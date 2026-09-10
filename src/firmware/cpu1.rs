//! CPU1 runtime composition.
//!
//! CPU1 owns the shared runtime I2C bus, radio, audio acquisition, and the
//! service tasks that consume that bus. Keeping the stack/executor and spawn
//! graph here makes second-core ownership explicit without changing any service
//! endpoint semantics.

use esp_hal::system::Stack;
use static_cell::StaticCell;

use crate::{
    platform::i2c as system_i2c,
    services::{audio, display::brightness as display_control, imu, network, touch},
    support::memory,
};

const STACK_SIZE: usize = 16 * 1024;

static STACK: StaticCell<Stack<STACK_SIZE>> = StaticCell::new();
static EXECUTOR: StaticCell<esp_rtos::embassy::Executor> = StaticCell::new();

pub(crate) fn init_stack() -> &'static mut Stack<STACK_SIZE> {
    let stack = STACK.init(Stack::new());
    memory::register_cpu1_stack(&mut *stack);
    stack
}

pub(crate) fn run(
    system_i2c: system_i2c::SystemI2cBlocking,
    audio_resources: audio::Resources,
    network_resources: network::Resources,
) {
    memory::init_cpu1_stack_watermark();
    let executor = EXECUTOR.init(esp_rtos::embassy::Executor::new());

    executor.run(move |spawner| {
        spawner.spawn(
            memory::cpu1_stack_monitor_task()
                .expect("Failed to allocate CPU1 stack monitor task"),
        );
        network::start(&spawner, network_resources, network::DEFAULT_CONFIG);

        let system_bus = system_i2c::into_async(system_i2c);
        spawner.spawn(
            display_control::task(system_bus)
                .expect("Failed to allocate CPU1 display-control task"),
        );
        spawner.spawn(
            imu::capture_task(system_bus, imu::DEFAULT_CONFIG)
                .expect("Failed to allocate CPU1 IMU task"),
        );
        spawner.spawn(
            touch::capture_task(system_bus).expect("Failed to allocate CPU1 touch task"),
        );
        spawner.spawn(
            audio::capture_task(audio_resources, spawner)
                .expect("Failed to allocate CPU1 audio task"),
        );
    });
}
