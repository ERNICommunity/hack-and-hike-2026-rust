use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::Channel,
    signal::Signal,
};
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;

use crate::{board, diagnostics, system_i2c::SystemI2cBus};

const FT6336_ADDR: u8 = 0x38;
const FT6336_TOUCH_DATA: u8 = 0x02;
const POLL_INTERVAL: Duration = Duration::from_millis(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TouchPoint {
    pub x: u16,
    pub y: u16,
}

#[derive(Clone, Copy, Debug)]
pub enum TouchEdge {
    Pressed(TouchPoint),
    Released(TouchPoint),
}

type EdgeChannel = Channel<CriticalSectionRawMutex, TouchEdge, 8>;
type PointSignal = Signal<CriticalSectionRawMutex, TouchPoint>;

// These outputs cross from CPU1 acquisition to CPU0 presentation, therefore
// they use CriticalSectionRawMutex. Only the physical I2C bus mutex is
// executor-local. The storage stays static for Embassy, while bootstrap hands
// each side an endpoint that references this concrete service instance.
struct Service {
    edges: EdgeChannel,
    latest_point: PointSignal,
}

impl Service {
    const fn new() -> Self {
        Self {
            edges: Channel::new(),
            latest_point: Signal::new(),
        }
    }
}

static SERVICE: StaticCell<Service> = StaticCell::new();

#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    service: &'static Service,
}

pub struct Input {
    service: &'static Service,
}

pub(crate) struct Endpoints {
    pub(crate) runtime: Runtime,
    pub(crate) input: Input,
}

pub(crate) fn init_endpoints() -> Endpoints {
    let service: &'static Service = SERVICE.init(Service::new());
    Endpoints {
        runtime: Runtime { service },
        input: Input { service },
    }
}

impl Input {
    pub fn next_edge(&mut self) -> Option<TouchEdge> {
        self.service.edges.try_receive().ok()
    }

    pub fn take_latest_point(&mut self) -> Option<TouchPoint> {
        self.service.latest_point.try_take()
    }
}

#[derive(Clone, Copy)]
enum TouchSample {
    Up,
    Down(TouchPoint),
    ReadError,
}

async fn read_sample(bus: SystemI2cBus) -> TouchSample {
    let mut data = [0u8; 5];

    let result = {
        let mut i2c = bus.lock().await;
        i2c.write_read_async(FT6336_ADDR, &[FT6336_TOUCH_DATA], &mut data)
            .await
    };

    if result.is_err() {
        return TouchSample::ReadError;
    }

    if data[0] & 0x0F == 0 {
        return TouchSample::Up;
    }

    let x = (u16::from(data[1] & 0x0F) << 8) | u16::from(data[2]);
    let y = (u16::from(data[3] & 0x0F) << 8) | u16::from(data[4]);

    if usize::from(x) >= board::DISPLAY_WIDTH || usize::from(y) >= board::DISPLAY_HEIGHT {
        return TouchSample::ReadError;
    }

    let (x, y) = board::logical_display_point(x, y);
    TouchSample::Down(TouchPoint { x, y })
}

/// CPU1 touch acquisition. This task never owns presentation state and never
/// waits for CPU0 to consume movement samples.
#[embassy_executor::task]
pub async fn capture_task(bus: SystemI2cBus, runtime: Runtime) {
    let mut pressed = false;
    let mut last_point = TouchPoint { x: 0, y: 0 };

    loop {
        match read_sample(bus).await {
            TouchSample::ReadError => diagnostics::record_touch_read_error(),
            TouchSample::Up if pressed => {
                pressed = false;
                if runtime
                    .service
                    .edges
                    .try_send(TouchEdge::Released(last_point))
                    .is_err()
                {
                    diagnostics::record_touch_edge_drop();
                }
            }
            TouchSample::Up => {}
            TouchSample::Down(point) if !pressed => {
                pressed = true;
                last_point = point;
                runtime.service.latest_point.signal(point);
                if runtime
                    .service
                    .edges
                    .try_send(TouchEdge::Pressed(point))
                    .is_err()
                {
                    diagnostics::record_touch_edge_drop();
                }
            }
            TouchSample::Down(point) if point != last_point => {
                last_point = point;
                runtime.service.latest_point.signal(point);
            }
            TouchSample::Down(_) => {}
        }

        Timer::after(POLL_INTERVAL).await;
    }
}
