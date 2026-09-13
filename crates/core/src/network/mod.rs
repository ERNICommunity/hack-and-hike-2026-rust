//! ESP-NOW wire protocol, typed messages and the peer table.
//!
//! Nothing here touches the radio; the firmware's network capability drives
//! these types from its CPU1 tasks.

pub mod message;
pub mod protocol;
pub mod state;
