//! Content node — the on-chain envelope for a single message slot.
//!
//! This module is decomposed out of `state.rs` so the *shape* of a message can
//! evolve independently from the rest of the protocol (threads, allocs, access,
//! fees, …).
//!
//! ## Design
//!
//! Rust has no class inheritance, so the "abstract base class + subclasses"
//! idea is expressed with composition + a trait:
//!
//! * [`ContentHeader`] is the **abstract base**: the metadata every message
//!   carries no matter its type (who, when, where, reply pointers).
//! * [`ContentNode`] is the **envelope** stored on chain: a [`ContentHeader`]
//!   plus a `kind` discriminator and an *opaque* `body` payload.
//! * [`ContentBody`] (in [`body`]) is the trait every concrete message type
//!   implements — the equivalent of "subclassing" the abstract base.
//!
//! ## Why the body is opaque
//!
//! `kind` is a forward-compatible `u16` discriminator and `body` is raw bytes
//! the program never has to understand. This is the SPL "type-length-value" /
//! discriminator pattern: brand-new message types can be introduced by clients
//! **after the program is deployed, with no program upgrade**, because the
//! program only needs to store the bytes, pay rent, charge fees, and close the
//! account — none of which depend on the body's internal structure.
//!
//! A Borsh `enum` of typed variants was deliberately avoided: Borsh fails to
//! deserialize an unknown enum discriminant, so adding a variant would force a
//! redeploy and could brick reads of newer accounts by an older program.

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::AccountInfo, program_error::ProgramError, pubkey::Pubkey,
};

use crate::error::ProtocolError;
use crate::state::{assert_owned_by, TAG_CONTENT};

mod body;
pub use body::*;

// ── content constants ──

pub const CONTENT_SEED: &[u8] = b"content";
pub const CONTENT_SLOTS: usize = 31;

/// Maximum length, in bytes, of a content node body regardless of its type.
pub const MAX_BODY_LEN: usize = 8192;

/// Back-compat alias: the body used to be a `text` field.
pub const MAX_TEXT_LEN: usize = MAX_BODY_LEN;

// ── abstract base ──

/// Metadata shared by every content node, independent of message type.
///
/// Think of this as the abstract base class: any [`ContentBody`] is attached to
/// exactly one header, and the header layout never changes when new message
/// types are added.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct ContentHeader {
    pub tag: u8,
    pub alloc_seq: u32,
    pub slot: u8,
    pub thread: Pubkey,
    pub author: Pubkey,
    pub created_at: i64,
    pub reply_alloc_seq: u32,
    pub reply_slot: u8,
}

impl ContentHeader {
    /// Fixed serialized size of the header (no variable-length fields).
    pub const SIZE: usize = 1 + 4 + 1 + 32 + 32 + 8 + 4 + 1;
}

// ── envelope ──

/// On-chain content node: a shared [`ContentHeader`] + a typed, opaque body.
///
/// The body is stored as raw bytes whose meaning is selected by `kind`
/// ([`ContentKind`]). The program treats `body` opaquely; only clients (or an
/// opt-in processor) decode it via the matching [`ContentBody`] implementation.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct ContentNode {
    pub header: ContentHeader,
    /// Message-type discriminator. Stored as a raw `u16` (not a Borsh enum) so
    /// unknown/future kinds still deserialize cleanly. Interpret via
    /// [`ContentKind::from_u16`].
    pub kind: u16,
    /// Opaque, type-specific payload. For [`ContentKind::Text`] this is the
    /// UTF-8 message bytes; for other kinds it is that type's encoding.
    pub body: Vec<u8>,
}

impl ContentNode {
    /// Serialized size for a node whose body is `body_len` bytes:
    /// header + `kind` (u16) + Borsh `Vec` length prefix (u32) + body.
    pub fn size(body_len: usize) -> usize {
        ContentHeader::SIZE + 2 + 4 + body_len
    }

    /// Builds a node from a typed body, validating it and encoding it into the
    /// opaque on-chain form. This is the recommended constructor.
    pub fn from_body<B: ContentBody>(
        header: ContentHeader,
        body: &B,
    ) -> Result<Self, ProgramError> {
        body.validate()?;
        Ok(Self {
            header,
            kind: B::KIND,
            body: body.to_body()?,
        })
    }

    /// Convenience accessor for the message-type discriminator.
    pub fn content_kind(&self) -> ContentKind {
        ContentKind::from_u16(self.kind)
    }

    /// Decodes the body as a specific concrete type. Errors if `kind` does not
    /// match `B`'s discriminator or the bytes are malformed.
    pub fn decode_body<B: ContentBody>(&self) -> Result<B, ProgramError> {
        if self.kind != B::KIND {
            return Err(ProtocolError::InvalidTag.into());
        }
        B::from_body(&self.body)
    }
}

// ── PDA derivation & loading ──

pub fn derive_content_pda(
    program_id: &Pubkey,
    thread: &Pubkey,
    alloc_seq: u32,
    slot: u8,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[CONTENT_SEED, thread.as_ref(), &alloc_seq.to_le_bytes(), &[slot]],
        program_id,
    )
}

pub fn load_content(program_id: &Pubkey, account: &AccountInfo) -> Result<ContentNode, ProgramError> {
    assert_owned_by(account, program_id)?;

    let content = ContentNode::try_from_slice(&account.data.borrow())
        .map_err(|_| ProtocolError::InvalidAccountData)?;

    if content.header.tag != TAG_CONTENT {
        return Err(ProtocolError::InvalidTag.into());
    }

    let (expected, _) = derive_content_pda(
        program_id,
        &content.header.thread,
        content.header.alloc_seq,
        content.header.slot,
    );

    if *account.key != expected {
        return Err(ProtocolError::InvalidPda.into());
    }

    Ok(content)
}
