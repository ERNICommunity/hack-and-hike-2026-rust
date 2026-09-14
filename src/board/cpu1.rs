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
    capabilities::{audio, backlight, imu, light, network, touch},
    platform::i2c,
};

/// Stack of every task on CPU1 together: the executor polls them all on it.
const STACK_SIZE: usize = 16 * 1024;

/// Memory for [`STACK_SIZE`]. It must outlive the core that runs on it, so it
/// lives in a `StaticCell` that hands out a `&'static mut` once.
static STACK: StaticCell<Stack<STACK_SIZE>> = StaticCell::new();
/// The async executor of CPU1. [`run`] never returns, so the executor is
/// stored for the whole program lifetime.
static EXECUTOR: StaticCell<esp_rtos::embassy::Executor> = StaticCell::new();

/// Everything CPU1 owns: its hardware and the runtime side of each capability.
pub(super) struct Cpu1 {
    /// The shared I2C bus of the power chip, IO expander, IMU and touch
    /// controller, still in blocking mode.
    pub(super) system_i2c: i2c::SystemI2cBlocking,
    /// I2S and its pins, for microphone and speaker.
    pub(super) audio_resources: audio::Resources,
    /// The Wi-Fi radio, for ESP-NOW.
    pub(super) network_resources: network::Resources,
    /// Queues shared with the microphone and speaker handles.
    pub(super) audio: audio::Runtime,
    /// Signal shared with the IMU handle.
    pub(super) imu: imu::Runtime,
    /// Queues shared with the network handle.
    pub(super) network: network::Runtime,
    /// Queue shared with the touch handle.
    pub(super) touch: touch::Runtime,
    /// Signal shared with the backlight handle.
    pub(super) backlight: backlight::Runtime,
    /// Signal shared with the light handle; `None` when no sensor answered.
    pub(super) light: Option<light::Runtime>,
}

/// Start the second core with its own async executor and the capability
/// tasks. Returns immediately; CPU1 runs from here on.
pub(super) fn start(cpu_ctrl: CPU_CTRL<'static>, interrupt: FROM_CPU_INTR1<'static>, cpu1: Cpu1) {
    let stack = STACK.init(Stack::new());
    esp_rtos::start_second_core(cpu_ctrl, interrupt, stack, move || run(cpu1));
}

/// The entry point of CPU1: spawn every capability task, then poll them
/// forever.
fn run(cpu1: Cpu1) {
    let executor = EXECUTOR.init(esp_rtos::embassy::Executor::new());

    executor.run(move |spawner| {
        network::spawn(
            &spawner,
            cpu1.network_resources,
            network::Config::default(),
            cpu1.network,
        );

        // The async I2C driver must be created on the core that services its
        // interrupt, so the blocking driver is converted here rather than on CPU0.
        let system_bus = i2c::into_async(cpu1.system_i2c);

        backlight::spawn(&spawner, system_bus, cpu1.backlight);
        imu::spawn(&spawner, system_bus, cpu1.imu);
        touch::spawn(&spawner, system_bus, cpu1.touch);
        if let Some(light) = cpu1.light {
            light::spawn(&spawner, system_bus, light);
        }
        audio::spawn(&spawner, cpu1.audio_resources, cpu1.audio);
    });
}
