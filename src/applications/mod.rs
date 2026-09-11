//! Stock applications built from hardware capabilities.

#[cfg(feature = "imu-worldview")]
pub(crate) mod imu_worldview;
#[cfg(feature = "mic-waveform")]
pub(crate) mod mic_waveform;
#[cfg(feature = "speaker-synth")]
pub(crate) mod speaker_synth;
