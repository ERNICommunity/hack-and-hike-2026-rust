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

/// A type that can be sent between boards.
///
/// On the radio, a message is the `postcard` encoding of the value, with
/// [`Message::KIND`] in the frame header in front of it. `postcard` is a
/// compact binary format for `serde` types.
pub trait Message: Serialize + DeserializeOwned {
    /// A name for this message type that is unique in the whole room, for
    /// example `"team-otters.hello"`. Two boards understand each other only
    /// when both use the same name for the same type.
    const NAME: &'static str;

    /// The kind sent on the radio: a 32-bit hash of [`Message::NAME`].
    const KIND: u32 = message_kind(Self::NAME);
}

/// 32-bit FNV-1a hash of a message name. FNV-1a (Fowler-Noll-Vo) is a
/// simple and fast hash function. This function is `const`, so the kind is
/// computed at compile time.
pub const fn message_kind(name: &str) -> u32 {
    /// The standard start value of 32-bit FNV-1a.
    const OFFSET_BASIS: u32 = 0x811C_9DC5;
    /// The standard prime factor of 32-bit FNV-1a.
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

/// Why a message could not be queued for sending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    /// The encoded message is larger than [`MAX_PAYLOAD`] bytes, so it does
    /// not fit into one radio frame.
    MessageTooLarge,
    /// The send queue is full. Try again on the next loop iteration.
    QueueFull,
    /// `Network::send_to` named a board that is not a peer now.
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

/// Why a received message could not be decoded into the requested type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The message has another type. Check it with [`IncomingMessage::is`]
    /// first.
    WrongKind,
    /// The kind matches, but the bytes are not a valid encoding of the type,
    /// or bytes are left over after decoding.
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

/// The `postcard` encoding of one message, at most [`MAX_PAYLOAD`] bytes.
#[doc(hidden)]
pub type Payload = ArrayVec<u8, MAX_PAYLOAD>;

/// A message that waits in the send queue until CPU1 sends it.
#[doc(hidden)]
pub struct OutgoingMessage {
    /// The board to send the message to. `None` sends it as a broadcast to
    /// every board in range.
    pub recipient: Option<DeviceId>,
    /// The [`Message::KIND`] of the message type.
    pub kind: u32,
    /// The encoded message.
    pub payload: Payload,
}

impl OutgoingMessage {
    /// Encode `value` for sending to `recipient`, or to every board when
    /// `recipient` is `None`.
    ///
    /// # Errors
    ///
    /// [`SendError::MessageTooLarge`] when the encoding is larger than
    /// [`MAX_PAYLOAD`] bytes.
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
    /// The board that sent the message. To reply, pass it to
    /// `Network::send_to`.
    pub sender: DeviceId,
    /// Which message type the payload holds; compared with [`Message::KIND`].
    kind: u32,
    /// The `postcard` encoding of the message.
    payload: Payload,
}

impl IncomingMessage {
    /// A received message from the payload bytes of an application frame.
    /// `None` when `bytes` is longer than [`MAX_PAYLOAD`].
    #[doc(hidden)]
    pub fn from_bytes(sender: DeviceId, kind: u32, bytes: &[u8]) -> Option<Self> {
        Some(Self {
            sender,
            kind,
            payload: Payload::try_from(bytes).ok()?,
        })
    }

    /// Whether this message has the kind of `T`.
    pub fn is<T: Message>(&self) -> bool {
        self.kind == T::KIND
    }

    /// Decode the message into a `T`.
    ///
    /// # Errors
    ///
    /// - [`DecodeError::WrongKind`] when the message has another kind.
    /// - [`DecodeError::Malformed`] when the bytes are not a valid `T`.
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
