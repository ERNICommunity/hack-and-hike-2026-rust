//! CPU0-owned reader capabilities for CPU1-produced data.
//!
//! The producer modules intentionally keep their service-specific primitives:
//! touch has an ordered edge channel plus a replace-latest point, IMU and
//! Network use replace-latest snapshots, and Audio exposes a latest complete PCM
//! block. `Cpu0Inputs` does not turn those into a generic event bus; it gives the
//! CPU0 owner a concrete, non-`Copy` capability for each contract.

use crate::{audio, imu, network, touch};

/// Complete set of CPU1→CPU0 data capabilities.
///
/// Bootstrap constructs this value once and moves each field to its sole CPU0
/// consumer. The types are deliberately not `Copy` or `Clone`, so normal Rust
/// moves make the intended ownership graph visible at construction time.
pub struct Cpu0Inputs {
    pub touch: TouchInput,
    pub imu: ImuInput,
    pub audio: AudioInput,
    pub network: NetworkInput,
}

impl Cpu0Inputs {
    pub const fn new() -> Self {
        Self {
            touch: TouchInput { _private: () },
            imu: ImuInput { _private: () },
            audio: AudioInput { _private: () },
            network: NetworkInput { _private: () },
        }
    }
}

/// Sole CPU0 consumer capability for touch presentation input.
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

/// Sole CPU0 consumer capability for fused IMU state.
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

/// Sole CPU0 consumer capability for complete stereo audio blocks.
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

/// Sole CPU0 consumer capability for the ESP-NOW presentation snapshot.
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
