#[cfg(any(feature = "mic", feature = "speaker"))]
pub(crate) mod audio;
#[cfg(feature = "camera")]
pub(crate) mod camera;
#[cfg(feature = "display")]
pub(crate) mod display;
#[cfg(feature = "imu")]
pub(crate) mod imu;
#[cfg(feature = "network")]
pub(crate) mod network;
#[cfg(feature = "touch")]
pub(crate) mod touch;
