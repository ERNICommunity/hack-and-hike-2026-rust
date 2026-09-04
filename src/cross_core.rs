//! Typed semantic boundary between CPU0 application code and CPU1 services.
//!
//! `Cpu0AppEndpoint` belongs with CPU0 application/model, Slint presentation,
//! and display coordination. `Cpu1ServiceEndpoint` belongs with CPU1 touch,
//! audio, future IMU, future ESP-NOW, and runtime system-I2C services.
//!
//! These are directional application-level lanes, not a generic event bus and
//! not CPU capability tokens. They make intended message direction explicit but
//! do not claim to prove which physical core is running.
//!
//! High-rate/service-specific data deliberately stays outside these lanes:
//! - touch press/release edges remain a bounded ordered channel in `touch`;
//! - touch movement remains a latest-value signal in `touch`;
//! - audio waveform data remains the specialized latest snapshot in `audio`.
//! Raw audio blocks, network packets, IMU sample streams, and hardware handles
//! do not belong in these semantic channels.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};

const COMMAND_CAPACITY: usize = 4;
const EVENT_CAPACITY: usize = 8;

/// Small semantic intent sent from the CPU0 application side to CPU1 services.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppCommand {
    SetImuEnabled(bool),
    SetTelemetryEnabled(bool),
}

/// Small semantic event sent from CPU1 services to the CPU0 application side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceEvent {
    AudioFault,
    ImuFault,
    NetworkFault,
}

static COMMANDS: Channel<CriticalSectionRawMutex, AppCommand, COMMAND_CAPACITY> = Channel::new();
static EVENTS: Channel<CriticalSectionRawMutex, ServiceEvent, EVENT_CAPACITY> = Channel::new();

/// CPU0-facing half of the semantic cross-core boundary.
///
/// The API exposes only CPU0's intended direction: send application commands
/// and receive service events.
pub struct Cpu0AppEndpoint {
    _private: (),
}

impl Cpu0AppEndpoint {
    pub fn try_send_command(&self, command: AppCommand) -> bool {
        COMMANDS.try_send(command).is_ok()
    }

    pub fn try_receive_event(&self) -> Option<ServiceEvent> {
        EVENTS.try_receive().ok()
    }
}

/// CPU1-facing half of the semantic cross-core boundary.
///
/// The API exposes only CPU1's intended direction: receive application commands
/// and send service events.
pub struct Cpu1ServiceEndpoint {
    _private: (),
}

impl Cpu1ServiceEndpoint {
    pub fn try_receive_command(&self) -> Option<AppCommand> {
        COMMANDS.try_receive().ok()
    }

    pub fn try_send_event(&self, event: ServiceEvent) -> bool {
        EVENTS.try_send(event).is_ok()
    }
}

/// Create the two typed views of the static semantic channels.
///
/// These endpoint values describe architectural roles; they are not CPU-affinity
/// capabilities.
pub fn split() -> (Cpu0AppEndpoint, Cpu1ServiceEndpoint) {
    (
        Cpu0AppEndpoint { _private: () },
        Cpu1ServiceEndpoint { _private: () },
    )
}
