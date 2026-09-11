//! Stock applications built from hardware capabilities.

#[cfg(feature = "camera-view")]
pub(crate) mod camera_view;
#[cfg(feature = "imu-worldview")]
pub(crate) mod imu_worldview;
#[cfg(feature = "mic-waveform")]
pub(crate) mod mic_waveform;
#[cfg(feature = "network-demo")]
pub(crate) mod network_demo;
#[cfg(feature = "speaker-synth")]
pub(crate) mod speaker_synth;
