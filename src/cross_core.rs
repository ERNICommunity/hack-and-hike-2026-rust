//! Typed semantic boundary between CPU0 application code and CPU1 services.
//!
//! The architecture is intentionally visible in the names of the endpoint and
//! message types:
//!
//! - [`Cpu0ApplicationCrossCoreEndpoint`] belongs with CPU0 application/model,
//!   Slint presentation, and display coordination.
//! - [`Cpu1PeripheralServicesCrossCoreEndpoint`] belongs with CPU1 touch,
//!   audio, future IMU, future ESP-NOW, and runtime system-I2C services.
//!
//! These are directional application-level lanes, not a generic event bus and
//! not CPU capability tokens. They express which side should send/receive each
//! semantic message; they do not claim to prove which physical core is running.
//!
//! High-rate/service-specific data deliberately stays outside these lanes:
//! - touch press/release edges remain a bounded ordered channel in `touch`;
//! - touch movement remains a latest-value signal in `touch`;
//! - audio waveform data remains the specialized latest snapshot in `audio`.
//! Raw audio blocks, network packets, IMU sample streams, and hardware handles
//! do not belong in these semantic channels.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};

const CPU0_TO_CPU1_APPLICATION_COMMAND_CAPACITY: usize = 4;
const CPU1_TO_CPU0_PERIPHERAL_EVENT_CAPACITY: usize = 8;

/// Small semantic intent sent from CPU0 application coordination to CPU1
/// peripheral services.
///
/// Keep variants compact. Bulk payloads belong in fixed storage/PSRAM with only
/// small semantic identifiers crossing the core boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cpu0ToCpu1ApplicationCommand {
    SetImuEnabled(bool),
    SetTelemetryEnabled(bool),
}

/// Small semantic event sent from CPU1 peripheral services to the CPU0
/// application/model side.
///
/// High-rate samples and transport payloads must use their specialized paths
/// instead of becoming variants here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cpu1ToCpu0PeripheralEvent {
    AudioFault,
    ImuFault,
    NetworkFault,
}

static CPU0_TO_CPU1_APPLICATION_COMMANDS: Channel<
    CriticalSectionRawMutex,
    Cpu0ToCpu1ApplicationCommand,
    CPU0_TO_CPU1_APPLICATION_COMMAND_CAPACITY,
> = Channel::new();

static CPU1_TO_CPU0_PERIPHERAL_EVENTS: Channel<
    CriticalSectionRawMutex,
    Cpu1ToCpu0PeripheralEvent,
    CPU1_TO_CPU0_PERIPHERAL_EVENT_CAPACITY,
> = Channel::new();

/// CPU0-facing half of the semantic cross-core boundary.
///
/// Its API exposes only CPU0's intended direction: send application commands
/// to CPU1 and receive semantic peripheral-service events from CPU1.
pub struct Cpu0ApplicationCrossCoreEndpoint {
    _private: (),
}

impl Cpu0ApplicationCrossCoreEndpoint {
    pub fn try_send_command_to_cpu1(&self, command: Cpu0ToCpu1ApplicationCommand) -> bool {
        CPU0_TO_CPU1_APPLICATION_COMMANDS
            .try_send(command)
            .is_ok()
    }

    pub fn try_receive_event_from_cpu1(&self) -> Option<Cpu1ToCpu0PeripheralEvent> {
        CPU1_TO_CPU0_PERIPHERAL_EVENTS.try_receive().ok()
    }
}

/// CPU1-facing half of the semantic cross-core boundary.
///
/// Its API exposes only CPU1's intended direction: receive application commands
/// from CPU0 and send semantic peripheral-service events back to CPU0.
pub struct Cpu1PeripheralServicesCrossCoreEndpoint {
    _private: (),
}

impl Cpu1PeripheralServicesCrossCoreEndpoint {
    pub fn try_receive_command_from_cpu0(&self) -> Option<Cpu0ToCpu1ApplicationCommand> {
        CPU0_TO_CPU1_APPLICATION_COMMANDS.try_receive().ok()
    }

    pub fn try_send_event_to_cpu0(&self, event: Cpu1ToCpu0PeripheralEvent) -> bool {
        CPU1_TO_CPU0_PERIPHERAL_EVENTS.try_send(event).is_ok()
    }
}

/// Create the two typed views of the static semantic channels.
///
/// The returned values are endpoint APIs, not proofs of CPU affinity. Their
/// verbose types make the intended placement and message direction explicit at
/// call sites without introducing capability-token machinery.
pub fn split_application_and_peripheral_service_endpoints() -> (
    Cpu0ApplicationCrossCoreEndpoint,
    Cpu1PeripheralServicesCrossCoreEndpoint,
) {
    (
        Cpu0ApplicationCrossCoreEndpoint { _private: () },
        Cpu1PeripheralServicesCrossCoreEndpoint { _private: () },
    )
}
