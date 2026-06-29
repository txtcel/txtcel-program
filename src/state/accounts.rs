//! On-chain account structures and their serialized-size helpers.
//!
//! Every persistent account the program owns is defined here as a Borsh struct
//! whose first field is a `tag` discriminator. The `size()` helpers mirror each
//! struct's Borsh layout exactly so account creation can request the right rent
//! up front; they are the single source of truth for each account's byte size.

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

use crate::content::CONTENT_SLOTS;

/// Allocation node: one record in a thread's dense, contiguously-numbered
/// sequence of slot-allocation pages. The chain carries no stored links: each
/// node is addressed solely by its PDA `[ALLOC_SEED, thread, alloc_seq]`, the
/// sequences run `0..=ThreadNode.last_alloc_seq` with no gaps (alloc nodes are
/// never closed), and `ThreadNode.last_alloc_seq` marks the tail and gates
/// growth. Content PDAs are addressed by `(thread, alloc_seq, slot)`.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct AllocNode {
    /// Account-type discriminator; always `TAG_ALLOC`.
    pub tag: u8,
    /// Owning thread/channel key this alloc belongs to; binds the node to its
    /// thread so allocs from other threads cannot be substituted.
    pub thread: Pubkey,
    /// This node's sequence number within the thread's alloc chain; part of the
    /// PDA seeds, so it also fixes the node's address.
    pub alloc_seq: u32,
}

impl AllocNode {
    /// Fixed serialized size; no variable-length fields.
    ///
    /// # Returns
    /// - Byte length of a serialized `AllocNode`.
    pub fn size() -> usize {
        1 + 32 + 4
    }
}

/// Thread/channel root: identity and per-thread fee policy. Its account key is
/// the channel id that all child PDAs (allocs, access, likes, follows) bind to.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct ThreadNode {
    /// Account-type discriminator; always `TAG_THREAD`.
    pub tag: u8,
    /// Number of alloc nodes created for this thread; tracks chain growth.
    pub alloc_count: u32,
    /// Sequence of the most recently created alloc node; the chain's tail.
    pub last_alloc_seq: u32,
    /// Wallet that created the thread; the only authority allowed to change its
    /// fee policy and the recipient of author-side fees.
    pub author: Pubkey,
    /// Fixed per-message fee in lamports a non-author pays to post in this
    /// thread. Set by the thread author. Split author/platform via
    /// `ProgramSettings.author_fee_cut_bps`.
    pub message_fee: u64,
    /// Fixed per-like fee in lamports a non-author pays to like content in this
    /// thread; split author/platform via `ProgramSettings.like_cut_bps`.
    pub like_fee: u64,
    /// UTF-8 channel title bytes (≤ `MAX_TITLE_LEN`); the only variable-length
    /// field, kept inline so a thread needs no extra metadata account.
    pub title: Vec<u8>,
}

impl ThreadNode {
    /// Serialized size for a given title length (the only variable field).
    ///
    /// # Parameters
    /// - `title_len` — byte length of the thread title to be stored.
    /// # Returns
    /// - Byte length of a serialized `ThreadNode` with that title.
    pub fn size(title_len: usize) -> usize {
        1 + 4 + 4 + 32 + 8 + 8 + 4 + title_len
    }
}

/// Singleton global config: who administers the program, where platform fees
/// go, and the basis-point cuts that drive every fee split.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct ProgramSettings {
    /// Account-type discriminator; always `TAG_SETTINGS`.
    pub tag: u8,
    /// Current program administrator; the only authority allowed to mutate
    /// these settings and sweep the treasury.
    pub admin: Pubkey,
    /// Destination wallet that treasury sweeps pay out to.
    pub treasury: Pubkey,
    /// Platform's cut, in basis points, of the rent paid when creating accounts
    /// (the "base fee"); `0` disables it.
    pub base_fee_bps: u32,
    /// Platform's share, in basis points, of a thread's per-message author fee.
    pub author_fee_cut_bps: u32,
    /// Platform's share, in basis points, of a thread's entry fee.
    pub entry_cut_bps: u32,
    /// Platform's share, in basis points, of a thread's per-like fee.
    pub like_cut_bps: u32,
}

impl ProgramSettings {
    /// Fixed serialized size; all fields are fixed-width.
    ///
    /// # Returns
    /// - Byte length of a serialized `ProgramSettings`.
    pub fn size() -> usize {
        1 + 32 + 32 + 4 * 4
    }
}

/// Per-thread access-control config: whether gating is enabled, who admins the
/// ACL, the entry fee, and a cached whitelist size for O(1) "is the whitelist
/// empty?" checks without scanning every `AccessEntry`.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct ThreadAccess {
    /// Account-type discriminator; always `TAG_ACCESS`.
    pub tag: u8,
    /// Thread this ACL config governs; binds the config to its thread.
    pub thread: Pubkey,
    /// Whether access gating is active; when `false`, posting is open to all.
    pub enabled: bool,
    /// Wallet authorized to manage this thread's ACL (whitelist/blacklist,
    /// entry fee, gating toggle).
    pub admin: Pubkey,
    /// Lamports a non-member must pay (via `request_access`) to gain membership;
    /// `0` means entry cannot be purchased, only granted by the admin.
    pub entry_fee: u64,
    /// Number of live `ACCESS_ALLOWED` membership entries (the whitelist). Lets
    /// the program know whether the whitelist is empty without scanning every
    /// `AccessEntry` PDA. When `enabled` is set, posting is restricted to
    /// members only if this is non-zero or an `entry_fee` is charged; an empty
    /// whitelist with no entry fee leaves the thread open to everyone except
    /// blacklisted wallets.
    pub whitelist_count: u32,
}

impl ThreadAccess {
    /// Fixed serialized size; all fields are fixed-width.
    ///
    /// # Returns
    /// - Byte length of a serialized `ThreadAccess`.
    pub fn size() -> usize {
        1 + 32 + 1 + 32 + 8 + 4
    }
}

/// Per-wallet membership record for thread access control.
/// PDA: [ACL_SEED, thread, wallet]. Status is ACCESS_ALLOWED or ACCESS_DENIED.
/// Existence + status give O(1) whitelist/blacklist checks with no size limit.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct AccessEntry {
    /// Account-type discriminator; always `TAG_ACCESS_ENTRY`.
    pub tag: u8,
    /// Thread this membership record applies to; part of the PDA seeds.
    pub thread: Pubkey,
    /// Wallet this record grants/denies access for; part of the PDA seeds.
    pub wallet: Pubkey,
    /// Membership state: `ACCESS_ALLOWED`, `ACCESS_DENIED`, or `ACCESS_FEE_EXEMPT`.
    pub status: u8,
}

impl AccessEntry {
    /// Fixed serialized size; all fields are fixed-width.
    ///
    /// # Returns
    /// - Byte length of a serialized `AccessEntry`.
    pub fn size() -> usize {
        1 + 32 + 32 + 1
    }
}

/// Per-alloc like counters: one count per content slot in the alloc, kept in a
/// dedicated account so liking a message never contends on the content PDA.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct AllocLikes {
    /// Account-type discriminator; always `TAG_LIKES`.
    pub tag: u8,
    /// Alloc sequence these counters belong to; binds the account to its alloc.
    pub alloc_seq: u32,
    /// Per-slot like counts, indexed by content slot within the alloc; a freed
    /// slot's entry is reset to zero so a reused slot never inherits old likes.
    pub counts: [u32; CONTENT_SLOTS],
}

impl AllocLikes {
    /// Fixed serialized size; the counts array length is a compile-time const.
    ///
    /// # Returns
    /// - Byte length of a serialized `AllocLikes`.
    pub fn size() -> usize {
        1 + 4 + 4 * CONTENT_SLOTS
    }
}

/// Per-wallet registry of followed channels. PDA: [FOLLOWS_SEED, owner].
/// Written only by its owner, so it is never a cross-user hot account. Stores
/// just channel addresses; titles/previews are resolved client-side by batching
/// the referenced `ThreadNode` accounts (kept off the registry to avoid
/// denormalized, stale data and extra rent).
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct FollowRegistry {
    /// Account-type discriminator; always `TAG_FOLLOW_REGISTRY`.
    pub tag: u8,
    /// Wallet that owns this registry; the only writer, part of the PDA seeds.
    pub owner: Pubkey,
    /// Channel keys the owner follows (≤ `MAX_FOLLOWS`); the account grows by
    /// one slot per follow and shrinks on unfollow.
    pub channels: Vec<Pubkey>,
}

impl FollowRegistry {
    /// Serialized size for a registry holding `channel_count` channels.
    ///
    /// # Parameters
    /// - `channel_count` — number of followed channel keys to be stored.
    /// # Returns
    /// - Byte length of a serialized `FollowRegistry` with that many channels.
    pub fn size(channel_count: usize) -> usize {
        1 + 32 + 4 + channel_count * 32
    }
}

/// One shard of a channel's follower counter.
/// PDA: [FOLLOWER_COUNT_SEED, thread, shard]. A follower writes to the shard
/// chosen deterministically from their wallet, so the total follower count is
/// `sum(shard.count for shard in 0..N_FOLLOWER_SHARDS)`.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct FollowerShard {
    /// Account-type discriminator; always `TAG_FOLLOWER_SHARD`.
    pub tag: u8,
    /// Channel whose follower count this shard contributes to; part of the seeds.
    pub thread: Pubkey,
    /// This shard's index (`0..N_FOLLOWER_SHARDS`); part of the PDA seeds.
    pub shard: u8,
    /// Followers counted by this shard; summed across shards for the live total.
    pub count: u64,
}

impl FollowerShard {
    /// Fixed serialized size; all fields are fixed-width.
    ///
    /// # Returns
    /// - Byte length of a serialized `FollowerShard`.
    pub fn size() -> usize {
        1 + 32 + 1 + 8
    }
}

// ── candidate slot (instruction data) ──

/// Caller-proposed `(alloc_seq, slot)` target for placing content. Passed in
/// instruction data (not a stored account) so the program can validate the
/// requested slot against the alloc chain.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct CandidateSlot {
    /// Alloc sequence the caller wants to place content under.
    pub alloc_seq: u32,
    /// Slot index within that alloc the caller wants to fill.
    pub slot: u8,
}
