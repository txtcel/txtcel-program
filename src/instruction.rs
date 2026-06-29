use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

use crate::state::CandidateSlot;

/// Wire-level command set of the program.
///
/// Each variant is the Borsh-encoded payload of one instruction; the entrypoint
/// decodes this enum and forwards the variant's fields to the matching
/// `process_*` handler. The variant discriminant (its order in this enum) is the
/// on-chain instruction selector, so variants must only ever be appended, never
/// reordered or removed.
#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub enum ProgramInstruction {
    /// Create a new thread (channel) together with its seq-0 allocation node.
    CreateRootAlloc {
        /// Fixed per-message author fee (lamports) charged to non-authors in the
        /// new thread; lets the creator monetize their channel from the start.
        message_fee: u64,
        /// Index of the treasury shard that collects the platform base fee on
        /// creation; sharding spreads writes so creators don't contend on one
        /// account.
        treasury_shard_idx: u16,
        /// Channel title bytes (opaque, length-bounded) stored in the thread.
        title: Vec<u8>,
    },
    /// Write a content element into the first usable candidate slot and collect
    /// its fees.
    FillSlot {
        /// Message-type discriminator stored verbatim so future kinds remain
        /// forward-compatible without a program upgrade.
        kind: u16,
        /// Opaque, type-specific payload bytes; only its length is bounded.
        body: Vec<u8>,
        /// Candidate (alloc_seq, slot) targets tried in order until a free one
        /// is found; lets clients race for a slot without prior coordination.
        candidates: Vec<CandidateSlot>,
        /// Treasury shard index that collects the platform base fee.
        treasury_shard_idx: u16,
        /// Author-fee shard index that collects the per-message author fee.
        author_fee_shard_idx: u8,
        /// Alloc sequence of the message being replied to (threading pointer).
        reply_alloc_seq: u32,
        /// Slot of the message being replied to (threading pointer).
        reply_slot: u8,
        /// Slippage cap: total fee must not exceed this, guarding against a fee
        /// change between submission and execution.
        max_fee: u64,
    },
    /// Link a fresh allocation node onto the tail of a thread's page chain.
    PrepareAlloc {
        /// Sequence of the current tail alloc being extended; must match the
        /// thread's `last_alloc_seq` so only the real tail can grow.
        alloc_seq: u32,
    },
    /// Drain excess lamports from treasury shards into the treasury wallet.
    SweepTreasury {
        /// Indices of the treasury shards to sweep, positionally paired with the
        /// remaining shard accounts so only the named PDAs are validated.
        shard_indices: Vec<u16>,
    },
    /// Drain excess lamports from a thread's author-fee shards to the author.
    SweepAuthorFees {
        /// Indices of the author-fee shards to sweep, positionally paired with
        /// the remaining shard accounts.
        shard_indices: Vec<u8>,
    },
    /// Close a program-owned account and refund its rent to the signer.
    CloseAccount,
    /// Initialize the singleton program settings account.
    InitSettings {
        /// Wallet that will receive swept platform revenue.
        treasury: Pubkey,
    },
    /// Change the treasury wallet in program settings.
    SetTreasury {
        /// New destination wallet for collected platform revenue.
        treasury: Pubkey,
    },
    /// Create a thread's access-control account, opting it into gating.
    InitThreadAccess {
        /// Whether gating starts enabled for the thread.
        enabled: bool,
        /// Treasury shard index that collects the rent fee for the new account.
        treasury_shard_idx: u16,
    },
    /// Toggle gating on an existing thread access account.
    SetThreadAccess {
        /// New enabled state for the thread's access control.
        enabled: bool,
    },
    /// Grant a wallet allow access by creating/flipping its `AccessEntry`.
    AddToWhitelist {
        /// Wallet being whitelisted.
        wallet: Pubkey,
    },
    /// Revoke a wallet's allow access by closing its `AccessEntry`.
    RemoveFromWhitelist {
        /// Wallet being removed from the whitelist.
        wallet: Pubkey,
    },
    /// Set a thread's fixed per-message author fee.
    SetMessageFee {
        /// New per-message fee in lamports.
        fee: u64,
    },
    /// Set the platform base fee charged on rent.
    SetBaseFee {
        /// New base fee in basis points.
        fee_bps: u32,
    },
    /// Set the platform's cut of the per-message author fee.
    SetAuthorFeeCut {
        /// New author-fee cut in basis points.
        fee_bps: u32,
    },
    /// Set the platform's cut of the entry fee.
    SetEntryCut {
        /// New entry-fee cut in basis points.
        fee_bps: u32,
    },
    /// Set the platform's cut of the like fee.
    SetLikeCut {
        /// New like-fee cut in basis points.
        fee_bps: u32,
    },
    /// Set a thread's per-like fee.
    SetLikeFee {
        /// New per-like fee in lamports.
        fee: u64,
    },
    /// Set a gated thread's entry fee.
    SetEntryFee {
        /// New entry fee in lamports.
        fee: u64,
    },
    /// Pay a gated thread's entry fee to join its whitelist.
    RequestAccess {
        /// Treasury shard index collecting the platform's entry-fee cut.
        treasury_shard_idx: u16,
        /// Author-fee shard index collecting the author's entry-fee share.
        author_fee_shard_idx: u8,
    },
    /// Like a content slot, bumping its counter and paying any like fee.
    LikeContent {
        /// Alloc sequence of the liked content (locates the slot).
        alloc_seq: u32,
        /// Slot index of the liked content within its alloc.
        slot: u8,
        /// Treasury shard index collecting the platform's like-fee cut.
        treasury_shard_idx: u16,
        /// Author-fee shard index collecting the author's like-fee share.
        author_fee_shard_idx: u8,
        /// Slippage cap on the like fee to guard against a mid-flight change.
        max_fee: u64,
    },
    /// Deny a wallet by creating/flipping its `AccessEntry`.
    AddToBlacklist {
        /// Wallet being blacklisted.
        wallet: Pubkey,
    },
    /// Lift a wallet's denial by closing its deny `AccessEntry`.
    RemoveFromBlacklist {
        /// Wallet being removed from the blacklist.
        wallet: Pubkey,
    },
    /// Append more bytes to an existing content slot's body within its window.
    AppendContent {
        /// Bytes appended to the slot's body.
        chunk: Vec<u8>,
        /// Treasury shard index collecting the base fee on the rent delta.
        treasury_shard_idx: u16,
        /// Author-fee shard index (validated for stable layout; not re-charged).
        author_fee_shard_idx: u8,
    },
    /// Transfer program admin rights to a new authority.
    SetAdmin {
        /// Wallet to become the new program admin.
        new_admin: Pubkey,
    },
    /// Mark a wallet fee-exempt by creating/flipping its `AccessEntry`.
    AddToFeeWhitelist {
        /// Wallet being granted fee-exempt access.
        wallet: Pubkey,
    },
    /// Revoke fee-exempt status by closing the wallet's `AccessEntry`.
    RemoveFromFeeWhitelist {
        /// Wallet being removed from the fee-exempt list.
        wallet: Pubkey,
    },
    /// Follow a channel: push its address into the caller's `FollowRegistry`
    /// and bump the channel's sharded follower counter.
    Subscribe,
    /// Unfollow a channel: remove its address from the caller's
    /// `FollowRegistry` and decrement the channel's sharded follower counter.
    Unsubscribe,
}
