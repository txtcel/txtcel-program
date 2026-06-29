//! Protocol-wide constants: account tags, ACL status codes, sizing/layout
//! bounds, sharding fan-out, fee caps, and PDA seeds.
//!
//! Centralizing these here keeps every magic number that defines the on-chain
//! wire format and PDA derivation in one audited place, so loaders, processors,
//! and the off-chain client all agree on the exact same values.

// ── account tags ──

/// First byte of a `ContentNode` account; identifies a stored message slot.
pub const TAG_CONTENT: u8 = 1;
/// First byte of an `AllocNode` account; identifies a slot-allocation node.
pub const TAG_ALLOC: u8 = 2;
/// First byte of a `ThreadNode` account; identifies a channel/thread root.
pub const TAG_THREAD: u8 = 3;
/// First byte of the singleton `ProgramSettings` account.
pub const TAG_SETTINGS: u8 = 5;
/// First byte of a `ThreadAccess` account; identifies a thread's ACL config.
pub const TAG_ACCESS: u8 = 6;
/// First byte of an `AllocLikes` account; identifies a per-alloc like counter.
pub const TAG_LIKES: u8 = 7;
/// First byte of an `AccessEntry` account; identifies a per-wallet ACL record.
pub const TAG_ACCESS_ENTRY: u8 = 9;
/// First byte of a `FollowRegistry` account; identifies a wallet's follow list.
pub const TAG_FOLLOW_REGISTRY: u8 = 10;
/// First byte of a `FollowerShard` account; identifies one follower-count shard.
pub const TAG_FOLLOWER_SHARD: u8 = 11;

// ── access entry status ──

/// `AccessEntry.status` value marking a wallet as whitelisted (may post).
pub const ACCESS_ALLOWED: u8 = 0;
/// `AccessEntry.status` value marking a wallet as blacklisted (may not post).
pub const ACCESS_DENIED: u8 = 1;
/// Member who is both allowed to post in a gated thread AND exempt from the
/// per-message author fee. Supersedes `ACCESS_ALLOWED`: a fee-exempt wallet is
/// always allowed, it just additionally pays no `ThreadNode.message_fee`.
pub const ACCESS_FEE_EXEMPT: u8 = 2;

// ── layout / sizing bounds ──

/// Maximum byte length of a thread title; bounds `ThreadNode` rent and growth.
pub const MAX_TITLE_LEN: usize = 64;
/// Number of treasury shards platform fees are spread across to avoid a single
/// write-contended treasury account.
pub const N_TREASURY_SHARDS: u16 = 512;
/// Number of per-thread author-fee shards, spreading author-fee writes the same
/// way the treasury is sharded.
pub const N_AUTHOR_FEE_SHARDS: u8 = 4;
/// Number of shards the per-channel follower counter is split across. Spreads
/// write contention of concurrent subscribes so the count is never a single
/// hot account; the live total is the sum of all shard counts.
pub const N_FOLLOWER_SHARDS: u8 = 8;
/// Upper bound on the channels one wallet can follow. Bounds the duplicate
/// scan compute and the registry account growth.
pub const MAX_FOLLOWS: usize = 1_000;
/// Hard ceiling (in basis points) on any configurable fee cut, so admin/author
/// settings can never claim more than 50% of an amount.
pub const MAX_FEE_CUT_BPS: u32 = 5_000;

// ── PDA seeds ──

/// Seed for the singleton `ProgramSettings` PDA.
pub const SETTINGS_SEED: &[u8] = b"settings";
/// Seed prefix for `AllocNode` PDAs, combined with thread + alloc sequence.
pub const ALLOC_SEED: &[u8] = b"alloc";
/// Seed prefix for the per-thread `ThreadAccess` PDA.
pub const ACCESS_SEED: &[u8] = b"access";
/// Seed prefix for per-wallet `AccessEntry` (ACL) PDAs.
pub const ACL_SEED: &[u8] = b"acl";
/// Seed prefix for per-alloc `AllocLikes` PDAs.
pub const LIKES_SEED: &[u8] = b"likes";
/// Seed prefix for treasury fee-shard PDAs, combined with the shard index.
pub const TREASURY_SHARD_SEED: &[u8] = b"treasury_shard";
/// Seed prefix for per-thread author-fee shard PDAs.
pub const AUTHOR_FEE_SEED: &[u8] = b"author_fee";
/// Seed prefix for a wallet's `FollowRegistry` PDA.
pub const FOLLOWS_SEED: &[u8] = b"follows";
/// Seed prefix for per-thread `FollowerShard` (follower-count) PDAs.
pub const FOLLOWER_COUNT_SEED: &[u8] = b"follower_count";
