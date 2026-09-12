#[cfg(any(
    feature = "display",
    feature = "touch",
    feature = "mic",
    feature = "speaker",
    feature = "camera",
))]
pub(crate) mod board;
#[cfg(any(
    feature = "display",
    feature = "touch",
    feature = "imu",
    feature = "mic",
    feature = "speaker",
    feature = "camera",
))]
pub(crate) mod i2c;
