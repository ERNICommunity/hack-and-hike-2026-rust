//! Start-up of CPU1, the second CPU core.
//!
//! CPU1 runs the timing-sensitive capability tasks: sensors, touch, audio,
//! radio and backlight control. Applications never use CPU1 directly. They
//! use the capability handles, which exchange data with the CPU1 tasks.

use esp_hal::{
    peripherals::{CPU_CTRL, FROM_CPU_INTR1},
    system::Stack,
};
use static_cell::StaticCell;

use crate::{
    board::i2c,
    capabilities::{audio, backlight, imu, light, network, proximity, touch},
};

/// Size of the one CPU1 stack. All CPU1 tasks share it, because one executor
/// runs them all on this stack.
const STACK_SIZE: usize = 16 * 1024;

/// Memory for the CPU1 stack. CPU1 uses it until the device stops, so it
/// must be `'static`. A `StaticCell` gives out one `&'static mut` reference
/// to it.
static STACK: StaticCell<Stack<STACK_SIZE>> = StaticCell::new();
/// The async executor of CPU1: the part of the async runtime that runs the
/// tasks. `Executor::run` needs a `&'static mut` reference and never returns,
/// so the executor is stored in a `StaticCell`.
static EXECUTOR: StaticCell<esp_rtos::embassy::Executor> = StaticCell::new();

/// Everything CPU1 owns: its hardware and the runtime side of each capability.
pub(super) struct Cpu1 {
    /// The shared system I2C bus, still in blocking mode. On CPU1, the
    /// backlight, IMU, touch and light tasks use it.
    pub(super) system_i2c: i2c::SystemI2cBlocking,
    /// The I2S controller, its DMA channel and its pins, for the microphones
    /// and the speaker. (I2S is the bus for audio samples.)
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
    /// Signals shared with the light and the proximity handles. One LTR-553
    /// task publishes to both. `None` when no sensor answered.
    pub(super) light: Option<(light::Runtime, proximity::Runtime)>,
}

/// Start the second core with its own async executor and the capability
/// tasks. Returns at once; CPU1 continues on its own.
///
/// # Panics
///
/// When it is called a second time.
pub(super) fn start(cpu_ctrl: CPU_CTRL<'static>, interrupt: FROM_CPU_INTR1<'static>, cpu1: Cpu1) {
    let stack = STACK.init(Stack::new());
    esp_rtos::start_second_core(cpu_ctrl, interrupt, stack, move || run(cpu1));
}

/// The entry point of CPU1: spawn every capability task, then run them
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

        // The async I2C driver must be created on the core that handles its
        // interrupt. So the blocking driver is converted here, not on CPU0.
        let system_bus = i2c::into_async(cpu1.system_i2c);

        backlight::spawn(&spawner, system_bus, cpu1.backlight);
        imu::spawn(&spawner, system_bus, cpu1.imu);
        touch::spawn(&spawner, system_bus, cpu1.touch);
        if let Some((light, proximity)) = cpu1.light {
            light::spawn(&spawner, system_bus, light, proximity);
        }
        audio::spawn(&spawner, cpu1.audio_resources, cpu1.audio);
    });
}
