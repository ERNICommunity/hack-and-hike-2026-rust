//! CPU1 runtime composition.
//!
//! CPU1 owns only the enabled runtime capabilities. The shared system I2C bus is
//! created when display brightness, IMU, or touch need it; radio and audio remain
//! independent concrete ownership paths.

use esp_hal::system::Stack;
use static_cell::StaticCell;

#[cfg(any(feature = "display", feature = "imu", feature = "touch"))]
use crate::platform::i2c as system_i2c;
#[cfg(any(feature = "mic", feature = "speaker"))]
use crate::services::audio;
#[cfg(feature = "display")]
use crate::services::display;
#[cfg(feature = "imu")]
use crate::services::imu;
#[cfg(feature = "network")]
use crate::services::network;
#[cfg(feature = "touch")]
use crate::services::touch;
use crate::support::memory;

const STACK_SIZE: usize = 16 * 1024;

static STACK: StaticCell<Stack<STACK_SIZE>> = StaticCell::new();
static EXECUTOR: StaticCell<esp_rtos::embassy::Executor> = StaticCell::new();

pub(super) struct ServiceEndpoints {
    #[cfg(any(feature = "mic", feature = "speaker"))]
    pub(super) audio: audio::Runtime,
    #[cfg(feature = "imu")]
    pub(super) imu: imu::Runtime,
    #[cfg(feature = "network")]
    pub(super) network: network::Runtime,
    #[cfg(feature = "touch")]
    pub(super) touch: touch::Runtime,
    #[cfg(feature = "display")]
    pub(super) display: display::BrightnessRuntime,
}

pub(super) fn init_stack() -> &'static mut Stack<STACK_SIZE> {
    let stack = STACK.init(Stack::new());
    memory::register_cpu1_stack(&mut *stack);
    stack
}

pub(super) fn run(
    #[cfg(any(feature = "display", feature = "imu", feature = "touch"))]
    system_i2c: system_i2c::SystemI2cBlocking,
    #[cfg(any(feature = "mic", feature = "speaker"))] audio_resources: audio::Resources,
    #[cfg(feature = "network")] network_resources: network::Resources,
    endpoints: ServiceEndpoints,
) {
    memory::init_cpu1_stack_watermark();
    let executor = EXECUTOR.init(esp_rtos::embassy::Executor::new());

    executor.run(move |spawner| {
        spawner.spawn(
            memory::cpu1_stack_monitor_task().expect("Failed to allocate CPU1 stack monitor task"),
        );

        #[cfg(feature = "network")]
        network::start(
            &spawner,
            network_resources,
            network::DEFAULT_CONFIG,
            endpoints.network,
        );

        #[cfg(any(feature = "display", feature = "imu", feature = "touch"))]
        let system_bus = system_i2c::into_async(system_i2c);

        #[cfg(feature = "display")]
        spawner.spawn(
            display::brightness_task(system_bus, endpoints.display)
                .expect("Failed to allocate CPU1 display-control task"),
        );
        #[cfg(feature = "imu")]
        spawner.spawn(
            imu::capture_task(system_bus, imu::DEFAULT_CONFIG, endpoints.imu)
                .expect("Failed to allocate CPU1 IMU task"),
        );
        #[cfg(feature = "touch")]
        spawner.spawn(
            touch::capture_task(system_bus, endpoints.touch)
                .expect("Failed to allocate CPU1 touch task"),
        );
        #[cfg(any(feature = "mic", feature = "speaker"))]
        spawner.spawn(
            audio::capture_task(audio_resources, spawner, endpoints.audio)
                .expect("Failed to allocate CPU1 audio task"),
        );
    });
}
