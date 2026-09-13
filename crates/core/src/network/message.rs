//! Typed application messages.
//!
//! Every board in the room shares one radio channel, so a board must be able
//! to tell its own application's messages from everyone else's. Each message
//! type therefore carries a *kind*: a 32-bit hash of the name the application
//! gives it. A payload is only decoded into a type whose kind matches.
//!
//! ```
//! use hack_and_hike_core::network::message::Message;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Serialize, Deserialize)]
//! struct Hello {
//!     number: u32,
//! }
//!
//! impl Message for Hello {
//!     // Make it unique to your application, not just to the type.
//!     const NAME: &'static str = "team-otters.hello";
//! }
//! ```

use core::fmt;

use arrayvec::ArrayVec;
use serde::{Serialize, de::DeserializeOwned};

use super::protocol::{DeviceId, MAX_PAYLOAD};

/// A type that can travel between boards.
///
/// The bytes on the radio are the `postcard` encoding of the value, preceded
/// by [`Message::KIND`], the hash of [`Message::NAME`].
pub trait Message: Serialize + DeserializeOwned {
    /// A name that identifies this message type across the whole room, for
    /// example `"team-otters.hello"`. Two boards only understand each other
    /// when they use the same name for the same type.
    const NAME: &'static str;

    /// The kind sent on the radio, derived from [`Message::NAME`].
    const KIND: u32 = message_kind(Self::NAME);
}

/// 32-bit FNV-1a hash of a message name. `const`, so the kind is computed at
/// compile time.
pub const fn message_kind(name: &str) -> u32 {
    const OFFSET_BASIS: u32 = 0x811C_9DC5;
    const PRIME: u32 = 0x0100_0193;

    let bytes = name.as_bytes();
    let mut hash = OFFSET_BASIS;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u32;
        hash = hash.wrapping_mul(PRIME);
        index += 1;
    }
    hash
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    /// The serialized message does not fit one radio frame ([`MAX_PAYLOAD`]).
    MessageTooLarge,
    /// The send queue is full; try again on the next loop iteration.
    QueueFull,
    /// `send_to` named a device that is not currently a peer.
    UnknownPeer,
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MessageTooLarge => write!(f, "message larger than {MAX_PAYLOAD} bytes"),
            Self::QueueFull => write!(f, "send queue is full"),
            Self::UnknownPeer => write!(f, "recipient is not a known peer"),
        }
    }
}

impl core::error::Error for SendError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The message is of another type; check [`IncomingMessage::is`] first.
    WrongKind,
    /// The kind matched, but the bytes are not a valid encoding of the type.
    Malformed,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongKind => write!(f, "message is of another kind"),
            Self::Malformed => write!(f, "message bytes are malformed"),
        }
    }
}

impl core::error::Error for DecodeError {}

#[doc(hidden)]
pub type Payload = ArrayVec<u8, MAX_PAYLOAD>;

/// A message waiting to be transmitted by CPU1.
#[doc(hidden)]
pub struct OutgoingMessage {
    /// `None` broadcasts to every peer.
    pub recipient: Option<DeviceId>,
    pub kind: u32,
    pub payload: Payload,
}

impl OutgoingMessage {
    pub fn new<T: Message>(recipient: Option<DeviceId>, value: &T) -> Result<Self, SendError> {
        let mut storage = [0u8; MAX_PAYLOAD];
        let encoded =
            postcard::to_slice(value, &mut storage).map_err(|_| SendError::MessageTooLarge)?;
        Ok(Self {
            recipient,
            kind: T::KIND,
            payload: Payload::try_from(&*encoded).map_err(|_| SendError::MessageTooLarge)?,
        })
    }
}

/// A message received from another board. Decode it into your own type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncomingMessage {
    pub sender: DeviceId,
    kind: u32,
    payload: Payload,
}

impl IncomingMessage {
    #[doc(hidden)]
    pub fn from_bytes(sender: DeviceId, kind: u32, bytes: &[u8]) -> Option<Self> {
        Some(Self {
            sender,
            kind,
            payload: Payload::try_from(bytes).ok()?,
        })
    }

    /// Whether this message is a `T`.
    pub fn is<T: Message>(&self) -> bool {
        self.kind == T::KIND
    }

    /// The message as a `T`. Fails when it is of another kind or malformed.
    pub fn decode<T: Message>(&self) -> Result<T, DecodeError> {
        if !self.is::<T>() {
            return Err(DecodeError::WrongKind);
        }
        let (value, rest) =
            postcard::take_from_bytes(&self.payload).map_err(|_| DecodeError::Malformed)?;
        // Unused trailing bytes mean the sender's type is not ours after all.
        if rest.is_empty() {
            Ok(value)
        } else {
            Err(DecodeError::Malformed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_kind_is_fnv1a() {
        // Reference values of 32-bit FNV-1a.
        assert_eq!(message_kind(""), 0x811C_9DC5);
        assert_eq!(message_kind("a"), 0xE40C_292C);
        assert_eq!(message_kind("foobar"), 0xBF9C_F968);
    }
}
