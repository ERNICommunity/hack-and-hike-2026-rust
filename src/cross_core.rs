//! Minimal semantic contracts between CPU0 and CPU1.
//!
//! # CPU ownership
//!
//! CPU0 owns application coordination, presentation, and display I/O.
//! CPU1 owns non-display peripheral services and timing-sensitive
//! acquisition/communication.
//!
//! Hardware handles never cross this boundary. Only small, bounded semantic
//! commands/events do.
//!
//! These lanes deliberately do **not** replace service-specific fast paths:
//! - touch press/release edges stay on their bounded ordered channel;
//! - touch movement stays latest-value;
//! - audio waveform data stays on the specialized latest-audio snapshot.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};

const APP_COMMAND_CAPACITY: usize = 4;
const APP_EVENT_CAPACITY: usize = 8;

/// Small CPU0 -> CPU1 application intent.
///
/// Keep variants semantic and compact. Bulk payloads belong in fixed storage or
/// PSRAM with only a small handle/index crossing cores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppCommand {
    SetImuEnabled(bool),
    SetTelemetryEnabled(bool),
}

/// Small CPU1 -> CPU0 semantic events.
///
/// High-rate samples and packet/audio payloads must not be added here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppEvent {
    AudioFault,
    ImuFault,
    NetworkFault,
}

static APP_COMMANDS: Channel<CriticalSectionRawMutex, AppCommand, APP_COMMAND_CAPACITY> =
    Channel::new();
static APP_EVENTS: Channel<CriticalSectionRawMutex, AppEvent, APP_EVENT_CAPACITY> =
    Channel::new();

pub fn try_send_command(command: AppCommand) -> bool {
    APP_COMMANDS.try_send(command).is_ok()
}

pub fn try_take_command() -> Option<AppCommand> {
    APP_COMMANDS.try_receive().ok()
}

pub fn try_send_event(event: AppEvent) -> bool {
    APP_EVENTS.try_send(event).is_ok()
}

pub fn try_take_event() -> Option<AppEvent> {
    APP_EVENTS.try_receive().ok()
}
