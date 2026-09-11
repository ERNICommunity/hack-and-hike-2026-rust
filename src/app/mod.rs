#[cfg(any(
    feature = "imu-worldview",
    feature = "mic-waveform",
    feature = "speaker-synth",
    feature = "network-demo",
    feature = "camera-view",
    feature = "settings",
    feature = "log-view",
))]
pub(crate) mod model;
#[cfg(any(
    feature = "imu-worldview",
    feature = "mic-waveform",
    feature = "speaker-synth",
    feature = "network-demo",
    feature = "camera-view",
    feature = "settings",
    feature = "log-view",
))]
pub(crate) mod ui;
