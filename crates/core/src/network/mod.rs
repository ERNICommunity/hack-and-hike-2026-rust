//! ESP-NOW wire protocol, typed messages and the peer table.
//!
//! ESP-NOW is Espressif's protocol for short Wi-Fi messages between nearby
//! boards, without a router. Nothing here uses the radio. The firmware's
//! network capability uses these types in its tasks on CPU1 (the second CPU
//! core).

pub mod message;
pub mod protocol;
pub mod state;
