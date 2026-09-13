//! CPU1 runtime.
//!
//! The second core runs the timing-sensitive capability tasks: sensors, touch,
//! audio, radio and backlight control. Applications never touch CPU1 directly;
//! they talk to it through the capability handles.

use esp_hal::{
    peripherals::{CPU_CTRL, FROM_CPU_INTR1},
    system::Stack,
};
use static_cell::StaticCell;

use crate::{
    capabilities::{audio, backlight, imu, network, touch},
    platform::i2c,
};

const STACK_SIZE: usize = 16 * 1024;

static STACK: StaticCell<Stack<STACK_SIZE>> = StaticCell::new();
static EXECUTOR: StaticCell<esp_rtos::embassy::Executor> = StaticCell::new();

/// Everything CPU1 owns: its hardware and the runtime side of each capability.
pub(super) struct Cpu1 {
    pub(super) system_i2c: i2c::SystemI2cBlocking,
    pub(super) audio_resources: audio::Resources,
    pub(super) network_resources: network::Resources,
    pub(super) audio: audio::Runtime,
    pub(super) imu: imu::Runtime,
    pub(super) network: network::Runtime,
    pub(super) touch: touch::Runtime,
    pub(super) backlight: backlight::Runtime,
}

pub(super) fn start(cpu_ctrl: CPU_CTRL<'static>, interrupt: FROM_CPU_INTR1<'static>, cpu1: Cpu1) {
    let stack = STACK.init(Stack::new());
    esp_rtos::start_second_core(cpu_ctrl, interrupt, stack, move || run(cpu1));
}

fn run(cpu1: Cpu1) {
    let executor = EXECUTOR.init(esp_rtos::embassy::Executor::new());

    executor.run(move |spawner| {
        network::start(
            &spawner,
            cpu1.network_resources,
            network::DEFAULT_CONFIG,
            cpu1.network,
        );

        // The async I2C driver must be created on the core that services its
        // interrupt, so the blocking driver is converted here rather than on CPU0.
        let system_bus = i2c::into_async(cpu1.system_i2c);

        spawner.spawn(
            backlight::task(system_bus, cpu1.backlight).expect("backlight task already spawned"),
        );
        spawner.spawn(
            imu::capture_task(system_bus, imu::DEFAULT_CONFIG, cpu1.imu)
                .expect("IMU task already spawned"),
        );
        spawner.spawn(
            touch::capture_task(system_bus, cpu1.touch).expect("touch task already spawned"),
        );
        spawner.spawn(
            audio::capture_task(cpu1.audio_resources, spawner, cpu1.audio)
                .expect("audio task already spawned"),
        );
    });
}
