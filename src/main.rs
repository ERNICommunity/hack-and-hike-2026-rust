#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

mod audio;
mod log;
mod screen;
mod system_i2c;
mod touch;

extern crate alloc;

use ::log::info;
use alloc::rc::Rc;
use bt_hci::controller::ExternalController;
use embassy_executor::Spawner;
use esp_backtrace as _;
use esp_hal::{clock::CpuClock, system::Stack, timer::timg::TimerGroup};
use esp_radio::ble::controller::BleConnector;
use slint::Model;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use static_cell::StaticCell;
use trouble_host::prelude::*;

const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 1;
const CPU1_STACK_SIZE: usize = 16 * 1024;
const WAVEFORM_POINTS: usize = 128;
const WAVEFORM_UPDATE_MS: u64 = 32;

static CPU1_STACK: StaticCell<Stack<CPU1_STACK_SIZE>> = StaticCell::new();
static CPU1_EXECUTOR: StaticCell<esp_rtos::embassy::Executor> = StaticCell::new();

/// Synchronize the persistent Slint log model with the logger snapshot without
/// rebuilding every SharedString on every refresh.
fn sync_log_model(model: &slint::VecModel<slint::SharedString>, logs: &str) {
    let old_count = model.row_count();

    if old_count == 0 {
        for line in logs.lines().rev() {
            model.push(slint::SharedString::from(line));
        }
        return;
    }

    let Some(old_newest) = model.row_data(old_count - 1) else {
        return;
    };

    let mut match_info = None;

    for (new_prefix_count, line) in logs.lines().enumerate() {
        if line != old_newest.as_str() {
            continue;
        }

        let mut overlap = 1usize;
        let mut old_index = old_count - 1;
        let mut valid = true;

        for older_line in logs.lines().skip(new_prefix_count + 1) {
            if old_index == 0 {
                break;
            }

            old_index -= 1;
            let Some(old_line) = model.row_data(old_index) else {
                valid = false;
                break;
            };

            if old_line.as_str() != older_line {
                valid = false;
                break;
            }

            overlap += 1;
        }

        if valid {
            match_info = Some((new_prefix_count, overlap));
            break;
        }
    }

    let Some((new_prefix_count, overlap)) = match_info else {
        model.clear();
        for line in logs.lines().rev() {
            model.push(slint::SharedString::from(line));
        }
        return;
    };

    for _ in 0..old_count.saturating_sub(overlap) {
        model.remove(0);
    }

    for index in (0..new_prefix_count).rev() {
        if let Some(line) = logs.lines().nth(index) {
            model.push(slint::SharedString::from(line));
        }
    }
}


/// Convert the newest 512-frame stereo PCM block into two 128-point display
/// waveforms. Four audio frames collapse into each display point; the sample
/// with the largest magnitude wins so short transients remain visible.
fn update_waveform_models(
    left_model: &slint::VecModel<f32>,
    right_model: &slint::VecModel<f32>,
    samples: &[i16; audio::BLOCK_SAMPLES],
    info: audio::AudioBlockInfo,
) {
    const FRAMES_PER_POINT: usize = audio::BLOCK_FRAMES / WAVEFORM_POINTS;

    // A small floor prevents near-silence from being amplified to full scale.
    // Each channel otherwise uses the peak of the current 32 ms block.
    let left_scale = info.peak_left.max(1024) as f32;
    let right_scale = info.peak_right.max(1024) as f32;

    for point in 0..WAVEFORM_POINTS {
        let first_frame = point * FRAMES_PER_POINT;
        let last_frame = first_frame + FRAMES_PER_POINT;

        let mut left_sample = 0i16;
        let mut right_sample = 0i16;
        let mut left_magnitude = 0u16;
        let mut right_magnitude = 0u16;

        for frame in first_frame..last_frame {
            let left = samples[frame * audio::CHANNELS];
            let right = samples[frame * audio::CHANNELS + 1];

            let left_abs = left.unsigned_abs();
            if left_abs > left_magnitude {
                left_magnitude = left_abs;
                left_sample = left;
            }

            let right_abs = right.unsigned_abs();
            if right_abs > right_magnitude {
                right_magnitude = right_abs;
                right_sample = right;
            }
        }

        let left_normalized = (left_sample as f32 / left_scale).clamp(-1.0, 1.0);
        let right_normalized = (right_sample as f32 / right_scale).clamp(-1.0, 1.0);

        left_model.set_row_data(point, left_normalized);
        right_model.set_row_data(point, right_normalized);
    }
}

#[derive(Clone, Copy)]
struct UiTouchState {
    pressed: bool,
    last_point: touch::TouchPoint,
}

impl UiTouchState {
    const fn new() -> Self {
        Self {
            pressed: false,
            last_point: touch::TouchPoint { x: 0, y: 0 },
        }
    }
}

fn logical_position(point: touch::TouchPoint) -> slint::LogicalPosition {
    slint::LogicalPosition {
        x: point.x as f32,
        y: point.y as f32,
    }
}

fn dispatch_move_if_changed(
    window: &MinimalSoftwareWindow,
    state: &mut UiTouchState,
    point: touch::TouchPoint,
) {
    if point.x == state.last_point.x && point.y == state.last_point.y {
        return;
    }

    state.last_point = point;
    window.dispatch_event(WindowEvent::PointerMoved {
        position: logical_position(point),
    });
}

/// CPU0-only bridge from plain cross-core touch messages into Slint events.
/// No Slint object is ever shared with CPU1.
fn dispatch_touch_input(window: &MinimalSoftwareWindow, state: &mut UiTouchState) {
    // Preserve press/release ordering. If CPU0 was busy rendering for the whole
    // gesture, synthesize a final move before release so Slint still observes
    // the drag displacement instead of seeing only a tap.
    while let Some(edge) = touch::try_take_edge() {
        match edge {
            touch::TouchEdge::Pressed(point) => {
                state.pressed = true;
                state.last_point = point;
                window.dispatch_event(WindowEvent::PointerPressed {
                    position: logical_position(point),
                    button: PointerEventButton::Left,
                });
            }
            touch::TouchEdge::Released(point) => {
                if state.pressed {
                    dispatch_move_if_changed(window, state, point);
                }

                window.dispatch_event(WindowEvent::PointerReleased {
                    position: logical_position(point),
                    button: PointerEventButton::Left,
                });
                state.pressed = false;
                state.last_point = point;
            }
        }
    }

    // Movement is intentionally latest-value only. This prevents a blocking
    // LCD render from creating a backlog of stale motion events.
    if let Some(point) = touch::take_latest_point() {
        if state.pressed {
            dispatch_move_if_changed(window, state, point);
        }
    }
}

// This creates a default app-descriptor required by the esp-idf bootloader.
esp_bootloader_esp_idf::esp_app_desc!();

slint::include_modules!();

struct McuPlatform {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for McuPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> core::time::Duration {
        core::time::Duration::from_millis(embassy_time::Instant::now().as_millis())
    }
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(_cpu0_spawner: Spawner) -> ! {
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);
    esp_alloc::heap_allocator!(size: 128 * 1024);

    log::init(::log::LevelFilter::Info);

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);

    // Start the RTOS scheduler on CPU0 first. start_second_core() requires it.
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    let mut screen_delay = esp_hal::delay::Delay::new();

    // One physical I2C0 controller, protected by one cross-core async mutex.
    let system_bus = system_i2c::init(peripherals.I2C0, peripherals.GPIO12, peripherals.GPIO11);

    // Startup hardware initialization happens before CPU1 acquisition tasks are
    // running, but still goes through the same physical-bus mutex. Screen gets
    // exclusive ownership of SPI2/LCD; it does NOT retain the I2C guard.
    let mut screen = {
        let mut i2c = system_bus.lock().await;

        let screen = screen::init(
            &mut *i2c,
            peripherals.SPI2,
            peripherals.DMA_CH1,
            peripherals.GPIO36,
            peripherals.GPIO37,
            peripherals.GPIO35,
            peripherals.GPIO3,
            &mut screen_delay,
        );

        audio::init_es7210(&mut *i2c, &mut screen_delay)
            .expect("Failed to initialize ES7210 microphone codec");

        screen
    };

    info!("==========================================");
    info!(">>> M5Stack CoreS3 Lite Booting Up! <<<");
    info!("==========================================");

    // Reserved / currently unused module GPIOs.
    let _ = peripherals.GPIO27;
    let _ = peripherals.GPIO28;
    let _ = peripherals.GPIO29;
    let _ = peripherals.GPIO30;
    let _ = peripherals.GPIO31;
    let _ = peripherals.GPIO32;

    let (mut _wifi_controller, _interfaces) =
        esp_radio::wifi::new(peripherals.WIFI, Default::default())
            .expect("Failed to initialize Wi-Fi controller");

    let transport = BleConnector::new(peripherals.BT, Default::default()).unwrap();
    let ble_controller = ExternalController::<_, 1>::new(transport);
    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let _ble_stack = trouble_host::new(ble_controller, &mut resources);

    info!("Starting CPU1 acquisition executor");

    let cpu1_stack = CPU1_STACK.init(Stack::new());

    // CPU1 owns hardware acquisition. The supplied function becomes the
    // second core's pinned RTOS main thread, and this Embassy executor runs
    // touch + I2S there. Task tokens are created on CPU1 so non-Send I2S state
    // never crosses cores.
    esp_rtos::start_second_core(
        peripherals.CPU_CTRL,
        sw_interrupt.software_interrupt1,
        cpu1_stack,
        move || {
            let executor = CPU1_EXECUTOR.init(esp_rtos::embassy::Executor::new());

            executor.run(move |cpu1_spawner| {
                cpu1_spawner.spawn(
                    touch::capture_task(system_bus).expect("Failed to allocate CPU1 touch task"),
                );

                cpu1_spawner.spawn(
                    audio::capture_task(
                        peripherals.I2S0,
                        peripherals.DMA_CH0,
                        peripherals.GPIO0,
                        peripherals.GPIO34,
                        peripherals.GPIO33,
                        peripherals.GPIO14,
                    )
                    .expect("Failed to allocate CPU1 audio task"),
                );
            });
        },
    );

    // Everything below this point is CPU0-only presentation state.
    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    window.set_size(slint::PhysicalSize::new(320, 240));

    slint::platform::set_platform(alloc::boxed::Box::new(McuPlatform {
        window: window.clone(),
    }))
    .expect("Failed to initialize Slint platform");

    let ui = AppWindow::new().unwrap();
    ui.show().unwrap();

    let log_model = Rc::new(slint::VecModel::<slint::SharedString>::default());
    ui.set_log_lines(log_model.clone().into());
    log::with_logs(|logs| sync_log_model(&log_model, logs));

    // Allocate the waveform models once. Updating rows in place lets Slint
    // dirty only the repeated waveform items instead of replacing models and
    // allocating 30 times per second.
    let mic_left_model = Rc::new(slint::VecModel::<f32>::default());
    let mic_right_model = Rc::new(slint::VecModel::<f32>::default());
    for _ in 0..WAVEFORM_POINTS {
        mic_left_model.push(0.0);
        mic_right_model.push(0.0);
    }
    ui.set_mic_left_samples(mic_left_model.clone().into());
    ui.set_mic_right_samples(mic_right_model.clone().into());

    let mut audio_samples = [0i16; audio::BLOCK_SAMPLES];
    let mut last_audio_sequence = 0u32;
    let mut last_waveform_update = embassy_time::Instant::now();

    let mut ui_touch = UiTouchState::new();
    let mut count = 0;
    let mut last_heartbeat = embassy_time::Instant::now();

    loop {
        let now = embassy_time::Instant::now();
        if now - last_heartbeat >= embassy_time::Duration::from_millis(350) && count < 100 {
            last_heartbeat = now;

            info!("Heartbeat count: {}", count);
            count += 1;
            log::with_logs(|logs| sync_log_model(&log_model, logs));
        }

        // Pull only the newest complete PCM block while the microphone page
        // is visible. No audio frames are queued for presentation: if CPU0 was
        // busy rendering, stale blocks are deliberately skipped.
        if ui.get_active_view() == 2
            && now - last_waveform_update
                >= embassy_time::Duration::from_millis(WAVEFORM_UPDATE_MS)
        {
            last_waveform_update = now;

            if let Some(info) = audio::copy_latest_interleaved(&mut audio_samples) {
                if info.sequence != last_audio_sequence {
                    last_audio_sequence = info.sequence;
                    update_waveform_models(
                        &mic_left_model,
                        &mic_right_model,
                        &audio_samples,
                        info,
                    );
                }
            }
        }

        // Consume CPU1 touch messages and translate them into Slint events on
        // CPU0. This is the only place touch meets Slint.
        dispatch_touch_input(&window, &mut ui_touch);

        slint::platform::update_timers_and_animations();

        // Screen is directly owned by CPU0. No display mutex, and no global
        // critical section around blocking SPI rendering.
        screen.render_slint_window(&window);

        embassy_time::Timer::after(embassy_time::Duration::from_millis(5)).await;
    }
}
