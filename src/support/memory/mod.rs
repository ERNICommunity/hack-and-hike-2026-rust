mod monitor;
mod psram;
mod stack;
#[cfg(any(feature = "ui", feature = "mic", feature = "camera"))]
pub(crate) mod storage;

#[cfg(feature = "app-stock")]
pub(crate) use monitor::HeapMonitor;
pub(crate) use monitor::report;
pub(crate) use psram::enable as enable_psram;
pub(crate) use stack::init_cpu0_stack_watermark;
#[cfg(any(
    feature = "display",
    feature = "touch",
    feature = "imu",
    feature = "mic",
    feature = "speaker",
    feature = "network",
))]
pub(crate) use stack::{
    cpu1_stack_monitor_task, init_cpu1_stack_watermark, register_cpu1_stack,
};
