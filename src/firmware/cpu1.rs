//! CPU1 runtime composition.
//!
//! CPU1 owns the shared runtime I2C bus, radio, audio acquisition, and the
//! service tasks that consume that bus. Keeping the stack/executor, runtime
//! ownership bundle, and spawn graph here makes second-core ownership explicit
//! without changing service transport semantics.

use esp_hal::system::Stack;
use static_cell::StaticCell;

use crate::{
    platform::i2c as system_i2c,
    services::{audio, display, imu, network, touch},
    support::memory,
};

const STACK_SIZE: usize = 16 * 1024;

static STACK: StaticCell<Stack<STACK_SIZE>> = StaticCell::new();
static EXECUTOR: StaticCell<esp_rtos::embassy::Executor> = StaticCell::new();

pub(crate) struct ServiceEndpoints {
    pub(crate) audio: audio::Runtime,
    pub(crate) imu: imu::Runtime,
    pub(crate) network: network::Runtime,
    pub(crate) touch: touch::Runtime,
    pub(crate) display: display::BrightnessRuntime,
}

/// Complete move-only ownership transferred into the second-core entry point.
///
/// Bootstrap constructs this only after the temporary board/camera I2C owners
/// have been destroyed and the final 400 kHz system-I2C driver exists. Moving
/// one value into the CPU1 closure makes it impossible for CPU0 composition to
/// retain any of these runtime resources accidentally.
pub(crate) struct RuntimeResources {
    system_i2c: system_i2c::SystemI2cBlocking,
    audio: audio::Resources,
    network: network::Resources,
    endpoints: ServiceEndpoints,
}

impl RuntimeResources {
    pub(crate) fn new(
        system_i2c: system_i2c::SystemI2cBlocking,
        audio: audio::Resources,
        network: network::Resources,
        endpoints: ServiceEndpoints,
    ) -> Self {
        Self {
            system_i2c,
            audio,
            network,
            endpoints,
        }
    }
}

pub(crate) fn init_stack() -> &'static mut Stack<STACK_SIZE> {
    let stack = STACK.init(Stack::new());
    memory::register_cpu1_stack(&mut *stack);
    stack
}

pub(crate) fn run(resources: RuntimeResources) {
    let RuntimeResources {
        system_i2c,
        audio: audio_resources,
        network: network_resources,
        endpoints,
    } = resources;

    memory::init_cpu1_stack_watermark();
    let executor = EXECUTOR.init(esp_rtos::embassy::Executor::new());
    let ServiceEndpoints {
        audio: audio_runtime,
        imu: imu_runtime,
        network: network_runtime,
        touch: touch_runtime,
        display: display_runtime,
    } = endpoints;

    executor.run(move |spawner| {
        spawner.spawn(
            memory::cpu1_stack_monitor_task()
                .expect("Failed to allocate CPU1 stack monitor task"),
        );
        network::start(
            &spawner,
            network_resources,
            network::DEFAULT_CONFIG,
            network_runtime,
        );

        let system_bus = system_i2c::into_async(system_i2c);
        spawner.spawn(
            display::brightness_task(system_bus, display_runtime)
                .expect("Failed to allocate CPU1 display-control task"),
        );
        spawner.spawn(
            imu::capture_task(system_bus, imu::DEFAULT_CONFIG, imu_runtime)
                .expect("Failed to allocate CPU1 IMU task"),
        );
        spawner.spawn(
            touch::capture_task(system_bus, touch_runtime)
                .expect("Failed to allocate CPU1 touch task"),
        );
        spawner.spawn(
            audio::capture_task(audio_resources, spawner, audio_runtime)
                .expect("Failed to allocate CPU1 audio task"),
        );
    });
}
