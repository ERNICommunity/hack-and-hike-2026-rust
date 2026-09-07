//! CPU0 reader handles for CPU1-produced data.
//!
//! The producer modules keep service-specific static synchronization primitives:
//! touch has an ordered edge channel plus a replace-latest point, IMU and network
//! use replace-latest snapshots, and audio exposes a latest complete PCM block.
//! These non-`Copy`, non-`Clone` handles make the intended CPU0 ownership graph
//! visible in ordinary Rust moves without pretending that the underlying static
//! primitives are linear capabilities. Constructing another `Cpu0Inputs` value
//! inside this crate would still address the same static producer state.

use crate::{audio, imu, network, touch};

/// Logical CPU0 readers for all CPU1-produced presentation data.
///
/// Bootstrap constructs one bundle and moves each field to its normal consumer.
/// The move-only wrappers prevent accidental duplication of a handle after that
/// point; uniqueness of construction remains an architectural convention because
/// the services themselves are backed by static Embassy primitives.
pub struct Cpu0Inputs {
    pub touch: TouchInput,
    pub imu: ImuInput,
    pub audio: AudioInput,
    pub network: NetworkInput,
}

impl Cpu0Inputs {
    pub(crate) const fn from_static_services() -> Self {
        Self {
            touch: TouchInput { _private: () },
            imu: ImuInput { _private: () },
            audio: AudioInput { _private: () },
            network: NetworkInput { _private: () },
        }
    }
}

/// CPU0 reader handle for touch presentation input.
///
/// Edge ordering is bounded by the producer's channel capacity; movement is
/// replace-latest and may be overwritten while CPU0 is busy rendering.
pub struct TouchInput {
    _private: (),
}

impl TouchInput {
    pub fn next_edge(&mut self) -> Option<touch::TouchEdge> {
        touch::try_take_edge()
    }

    pub fn take_latest_point(&mut self) -> Option<touch::TouchPoint> {
        touch::take_latest_point()
    }
}

/// CPU0 reader handle for fused IMU state.
///
/// Multiple CPU1 publications collapse to the newest `imu::Snapshot`.
pub struct ImuInput {
    _private: (),
}

impl ImuInput {
    pub fn take_latest(&mut self) -> Option<imu::Snapshot> {
        imu::take_latest()
    }
}

/// CPU0 reader handle for complete stereo audio blocks.
///
/// The read is non-blocking. If CPU1 is publishing at the same instant, CPU0
/// skips that presentation tick instead of waiting on the producer.
pub struct AudioInput {
    _private: (),
}

impl AudioInput {
    pub fn copy_latest_interleaved(
        &mut self,
        out: &mut [i16; audio::BLOCK_SAMPLES],
    ) -> Option<audio::AudioBlockInfo> {
        audio::copy_latest_interleaved(out)
    }
}

/// CPU0 reader handle for the ESP-NOW presentation snapshot.
///
/// Multiple radio updates collapse to the newest bounded peer-table snapshot.
pub struct NetworkInput {
    _private: (),
}

impl NetworkInput {
    pub fn take_latest(&mut self) -> Option<network::Snapshot> {
        network::take_latest()
    }
}
