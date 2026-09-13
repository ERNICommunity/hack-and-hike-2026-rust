//! Typed application messages at the network capability boundary.

use arrayvec::ArrayVec;
use serde::{Serialize, de::DeserializeOwned};

use super::protocol::{DeviceId, MAX_PAYLOAD};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    /// The serialized message does not fit one radio frame ([`MAX_PAYLOAD`]).
    MessageTooLarge,
    /// The send queue is full; try again on the next loop iteration.
    QueueFull,
    /// `send_to` named a device that is not currently a peer.
    UnknownPeer,
}

/// The payload was not a valid encoding of the requested type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeError;

pub type Payload = ArrayVec<u8, MAX_PAYLOAD>;

pub fn serialize_payload<T: Serialize>(value: &T) -> Result<Payload, SendError> {
    let mut storage = [0u8; MAX_PAYLOAD];
    let encoded =
        postcard::to_slice(value, &mut storage).map_err(|_| SendError::MessageTooLarge)?;
    Payload::try_from(&*encoded).map_err(|_| SendError::MessageTooLarge)
}

/// A message waiting to be transmitted by CPU1.
pub struct OutgoingMessage {
    /// `None` broadcasts to every peer.
    pub recipient: Option<DeviceId>,
    pub payload: Payload,
}

/// A message received from another device. Decode it into your own type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncomingMessage {
    pub sender: DeviceId,
    payload: Payload,
}

impl IncomingMessage {
    pub fn from_bytes(sender: DeviceId, bytes: &[u8]) -> Option<Self> {
        Some(Self {
            sender,
            payload: Payload::try_from(bytes).ok()?,
        })
    }

    pub fn decode<T>(&self) -> Result<T, DecodeError>
    where
        T: DeserializeOwned,
    {
        postcard::from_bytes(self.payload.as_slice()).map_err(|_| DecodeError)
    }
}
