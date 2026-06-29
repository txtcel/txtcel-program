//! Message bodies — the "subclasses" of the abstract content node.
//!
//! Every concrete message type implements [`ContentBody`], which is the
//! abstract interface a body must satisfy to be stored in a [`super::ContentNode`].
//! The on-chain program only ever needs `KIND`, `to_body`, and `validate`; full
//! decoding (`from_body`) is mostly for clients and tests.
//!
//! ## Adding a new message type after release
//!
//! Because the node body is opaque to the program, a new type usually needs **no
//! program upgrade** — pick a free `kind` discriminator and define the body on
//! the client. If you also want the type available on-chain (for validation or
//! type-aware fees), add it here:
//!
//! ```ignore
//! use borsh::{BorshDeserialize, BorshSerialize};
//!
//! pub const KIND_IMAGE: u16 = 1;
//!
//! #[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
//! pub struct ImageBody {
//!     pub uri: Vec<u8>,
//!     pub width: u32,
//!     pub height: u32,
//! }
//!
//! // Struct-shaped bodies just opt into Borsh encoding and get `ContentBody`
//! // for free via the blanket impl below.
//! impl BorshContentBody for ImageBody {
//!     const KIND: u16 = KIND_IMAGE;
//! }
//! ```
//!
//! Then register the discriminator in [`ContentKind`].

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::program_error::ProgramError;

use crate::error::ProtocolError;

use super::MAX_BODY_LEN;

// ── known discriminators ──

/// Plain UTF-8 text message. The original (and default) content type.
pub const KIND_TEXT: u16 = 0;

/// Known message-type discriminators.
///
/// `Unknown(raw)` is the forward-compatibility escape hatch: a node written by a
/// newer client carries a `kind` this build doesn't recognise yet, and it still
/// round-trips instead of failing to deserialize.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentKind {
    /// Plain UTF-8 text — the default, always-supported message type.
    Text,
    /// Any discriminator this build does not recognise; the wrapped `u16` keeps
    /// the original raw value so future kinds round-trip instead of failing.
    Unknown(u16),
}

impl ContentKind {
    /// Parses a raw discriminator into a known kind or the `Unknown` fallback.
    ///
    /// # Parameters
    /// - `value` — the raw `u16` discriminator from a node's `kind`.
    ///
    /// # Returns
    /// - The matching `ContentKind`, or `Unknown(value)` if unrecognised.
    pub fn from_u16(value: u16) -> Self {
        match value {
            KIND_TEXT => ContentKind::Text,
            other => ContentKind::Unknown(other),
        }
    }

    /// Converts the kind back to the raw discriminator stored on chain.
    ///
    /// # Parameters
    /// - `self` — the kind to encode.
    ///
    /// # Returns
    /// - The `u16` discriminator (the original raw value for `Unknown`).
    pub fn to_u16(self) -> u16 {
        match self {
            ContentKind::Text => KIND_TEXT,
            ContentKind::Unknown(value) => value,
        }
    }

    /// Whether this build understands the discriminator.
    ///
    /// # Parameters
    /// - `self` — the kind to test.
    ///
    /// # Returns
    /// - `true` for recognised kinds, `false` for `Unknown`.
    pub fn is_known(self) -> bool {
        !matches!(self, ContentKind::Unknown(_))
    }
}

// ── abstract body interface ──

/// The abstract interface every concrete message type implements.
///
/// `to_body`/`from_body` convert to and from the raw bytes stored in
/// [`super::ContentNode::body`]. `validate` enforces type-specific invariants
/// (size, structure, …) before a node is written.
pub trait ContentBody: Sized {
    /// Discriminator written into [`super::ContentNode::kind`].
    const KIND: u16;

    /// Encode into the opaque on-chain body bytes.
    ///
    /// # Parameters
    /// - `self` — the typed body to serialize.
    ///
    /// # Returns
    /// - `Ok(bytes)` to store in the node, or an encoding error.
    fn to_body(&self) -> Result<Vec<u8>, ProgramError>;

    /// Decode from the opaque on-chain body bytes.
    ///
    /// # Parameters
    /// - `body` — the raw on-chain payload bytes.
    ///
    /// # Returns
    /// - `Ok(Self)` on success, or a decode error if the bytes are malformed.
    fn from_body(body: &[u8]) -> Result<Self, ProgramError>;

    /// Type-specific validation run before the node is persisted.
    ///
    /// # Parameters
    /// - `self` — the body whose invariants are checked.
    ///
    /// # Returns
    /// - `Ok(())` if valid, otherwise a type-specific error.
    fn validate(&self) -> Result<(), ProgramError>;
}

/// Convenience trait for struct-shaped bodies whose wire form is simply their
/// Borsh encoding. Implementors get [`ContentBody`] for free via the blanket
/// impl below; override [`BorshContentBody::validate`] for custom checks.
pub trait BorshContentBody: BorshSerialize + BorshDeserialize + Sized {
    /// Discriminator written into [`super::ContentNode::kind`].
    const KIND: u16;

    /// Optional type-specific validation; defaults to accepting everything.
    ///
    /// # Parameters
    /// - `self` — the body to check.
    ///
    /// # Returns
    /// - `Ok(())` by default; override to enforce custom invariants.
    fn validate(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}

impl<T: BorshContentBody> ContentBody for T {
    const KIND: u16 = <T as BorshContentBody>::KIND;

    /// Encodes the struct via its Borsh representation.
    ///
    /// # Parameters
    /// - `self` — the body to serialize.
    ///
    /// # Returns
    /// - `Ok(bytes)`, or `ProtocolError::InvalidAccountData` on encode failure.
    fn to_body(&self) -> Result<Vec<u8>, ProgramError> {
        borsh::to_vec(self).map_err(|_| ProtocolError::InvalidAccountData.into())
    }

    /// Decodes the struct from its Borsh representation.
    ///
    /// # Parameters
    /// - `body` — the raw on-chain payload bytes.
    ///
    /// # Returns
    /// - `Ok(Self)`, or `ProtocolError::InvalidAccountData` on malformed bytes.
    fn from_body(body: &[u8]) -> Result<Self, ProgramError> {
        Self::try_from_slice(body).map_err(|_| ProtocolError::InvalidAccountData.into())
    }

    /// Delegates to the inherent [`BorshContentBody::validate`] implementation.
    ///
    /// # Parameters
    /// - `self` — the body to check.
    ///
    /// # Returns
    /// - Whatever the concrete `BorshContentBody::validate` returns.
    fn validate(&self) -> Result<(), ProgramError> {
        <T as BorshContentBody>::validate(self)
    }
}

// ── default body: plain text ──

/// The original message type: raw UTF-8 bytes.
///
/// Encoded as the bytes themselves (no extra Borsh framing) so the on-chain
/// `body` is identical to the legacy `text` field's contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextBody {
    /// Raw UTF-8 message bytes, stored verbatim as the node body.
    pub text: Vec<u8>,
}

impl TextBody {
    /// Wraps raw bytes as a text body.
    ///
    /// # Parameters
    /// - `text` — the UTF-8 message bytes to carry.
    ///
    /// # Returns
    /// - A `TextBody` owning `text` (not validated until [`ContentBody::validate`]).
    pub fn new(text: Vec<u8>) -> Self {
        Self { text }
    }
}

impl ContentBody for TextBody {
    const KIND: u16 = KIND_TEXT;

    /// Returns the message bytes unchanged (no extra Borsh framing).
    ///
    /// # Parameters
    /// - `self` — the text body to encode.
    ///
    /// # Returns
    /// - `Ok` with a clone of the raw `text` bytes.
    fn to_body(&self) -> Result<Vec<u8>, ProgramError> {
        Ok(self.text.clone())
    }

    /// Wraps the raw on-chain bytes back into a `TextBody`.
    ///
    /// # Parameters
    /// - `body` — the raw on-chain payload bytes.
    ///
    /// # Returns
    /// - `Ok(TextBody)` owning a copy of `body`.
    fn from_body(body: &[u8]) -> Result<Self, ProgramError> {
        Ok(Self { text: body.to_vec() })
    }

    /// Enforces the maximum body length.
    ///
    /// # Parameters
    /// - `self` — the text body to check.
    ///
    /// # Returns
    /// - `Ok(())` if within `MAX_BODY_LEN`, else `ProtocolError::TextTooLong`.
    fn validate(&self) -> Result<(), ProgramError> {
        if self.text.len() > MAX_BODY_LEN {
            return Err(ProtocolError::TextTooLong.into());
        }
        Ok(())
    }
}
