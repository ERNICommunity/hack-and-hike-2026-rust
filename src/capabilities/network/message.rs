//! Typed application-message serialization at the network capability boundary.

use arrayvec::ArrayVec;
use serde::{Serialize, de::DeserializeOwned};

use super::{DeviceId, MAX_PAYLOAD};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    MessageTooLarge,
    QueueFull,
    UnknownPeer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeError;

pub(super) fn serialize_payload<T: Serialize>(
    value: &T,
) -> Result<ArrayVec<u8, MAX_PAYLOAD>, SendError> {
    let mut storage = [0u8; MAX_PAYLOAD];
    let encoded =
        postcard::to_slice(value, &mut storage).map_err(|_| SendError::MessageTooLarge)?;
    let mut payload = ArrayVec::new();
    payload
        .try_extend_from_slice(encoded)
        .map_err(|_| SendError::MessageTooLarge)?;
    Ok(payload)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncomingMessage {
    pub sender: DeviceId,
    pub recipient: Option<DeviceId>,
    payload: ArrayVec<u8, MAX_PAYLOAD>,
}

impl IncomingMessage {
    pub(super) fn from_bytes(
        sender: DeviceId,
        recipient: Option<DeviceId>,
        bytes: &[u8],
    ) -> Option<Self> {
        let mut payload = ArrayVec::new();
        payload.try_extend_from_slice(bytes).ok()?;
        Some(Self {
            sender,
            recipient,
            payload,
        })
    }

    pub fn decode<T>(&self) -> Result<T, DecodeError>
    where
        T: DeserializeOwned,
    {
        postcard::from_bytes(self.payload.as_slice()).map_err(|_| DecodeError)
    }

    pub fn payload_len(&self) -> usize {
        self.payload.len()
    }
}
