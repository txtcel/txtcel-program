//! Canonical PDA derivations for every account type.
//!
//! Each `derive_*_pda` wraps `find_program_address` with the exact seed layout
//! for one account kind, so processors, loaders, and the off-chain client all
//! derive identical addresses and bumps from the same source. Changing a seed
//! here changes the on-chain address space, so these are effectively part of
//! the protocol's wire format.

use solana_program::pubkey::Pubkey;

use super::constants::{
    ACCESS_SEED, ACL_SEED, ALLOC_SEED, AUTHOR_FEE_SEED, FOLLOWER_COUNT_SEED, FOLLOWS_SEED,
    LIKES_SEED, N_FOLLOWER_SHARDS, SETTINGS_SEED, TREASURY_SHARD_SEED,
};

/// Derives the singleton `ProgramSettings` PDA (one per program).
///
/// # Parameters
/// - `program_id` — this program's id, used as the deriving program.
/// # Returns
/// - `(settings_pda, bump)` — the canonical address and its bump.
pub fn derive_settings_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[SETTINGS_SEED], program_id)
}

/// Derives the `AllocNode` PDA for a given thread and alloc sequence number.
///
/// # Parameters
/// - `program_id` — this program's id.
/// - `thread` — the owning thread/channel key.
/// - `alloc_seq` — the alloc node's sequence number within the thread.
/// # Returns
/// - `(alloc_pda, bump)` — the canonical address and its bump.
pub fn derive_alloc_pda(program_id: &Pubkey, thread: &Pubkey, alloc_seq: u32) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[ALLOC_SEED, thread.as_ref(), &alloc_seq.to_le_bytes()], program_id)
}

/// Derives the per-thread `ThreadAccess` PDA holding the thread's ACL config.
///
/// # Parameters
/// - `program_id` — this program's id.
/// - `thread` — the thread whose access config is addressed.
/// # Returns
/// - `(access_pda, bump)` — the canonical address and its bump.
pub fn derive_access_pda(program_id: &Pubkey, thread: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[ACCESS_SEED, thread.as_ref()], program_id)
}

/// Derives the per-wallet `AccessEntry` PDA used for O(1) ACL membership checks.
///
/// # Parameters
/// - `program_id` — this program's id.
/// - `thread` — the thread the membership applies to.
/// - `wallet` — the wallet the membership record is for.
/// # Returns
/// - `(access_entry_pda, bump)` — the canonical address and its bump.
pub fn derive_access_entry_pda(
    program_id: &Pubkey,
    thread: &Pubkey,
    wallet: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[ACL_SEED, thread.as_ref(), wallet.as_ref()], program_id)
}

/// Derives the `AllocLikes` PDA holding like counters for a thread's alloc.
///
/// # Parameters
/// - `program_id` — this program's id.
/// - `thread` — the owning thread/channel key.
/// - `alloc_seq` — the alloc sequence the like counters belong to.
/// # Returns
/// - `(likes_pda, bump)` — the canonical address and its bump.
pub fn derive_likes_pda(program_id: &Pubkey, thread: &Pubkey, alloc_seq: u32) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[LIKES_SEED, thread.as_ref(), &alloc_seq.to_le_bytes()], program_id)
}

/// Derives one treasury fee-shard PDA by shard index (0..`N_TREASURY_SHARDS`).
///
/// # Parameters
/// - `program_id` — this program's id.
/// - `shard` — the treasury shard index.
/// # Returns
/// - `(treasury_shard_pda, bump)` — the canonical address and its bump.
pub fn derive_treasury_shard_pda(program_id: &Pubkey, shard: u16) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[TREASURY_SHARD_SEED, &shard.to_le_bytes()], program_id)
}

/// Derives one per-thread author-fee shard PDA by shard index.
///
/// # Parameters
/// - `program_id` — this program's id.
/// - `thread` — the thread whose author fees the shard collects.
/// - `shard` — the author-fee shard index (`0..N_AUTHOR_FEE_SHARDS`).
/// # Returns
/// - `(author_fee_pda, bump)` — the canonical address and its bump.
pub fn derive_author_fee_pda(program_id: &Pubkey, thread: &Pubkey, shard: u8) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[AUTHOR_FEE_SEED, thread.as_ref(), &[shard]], program_id)
}

/// Derives a wallet's `FollowRegistry` PDA (its personal follow list).
///
/// # Parameters
/// - `program_id` — this program's id.
/// - `owner` — the wallet that owns the follow registry.
/// # Returns
/// - `(follow_registry_pda, bump)` — the canonical address and its bump.
pub fn derive_follow_registry_pda(program_id: &Pubkey, owner: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[FOLLOWS_SEED, owner.as_ref()], program_id)
}

/// Derives one per-thread `FollowerShard` PDA by shard index.
///
/// # Parameters
/// - `program_id` — this program's id.
/// - `thread` — the channel whose follower count the shard contributes to.
/// - `shard` — the follower-count shard index (`0..N_FOLLOWER_SHARDS`).
/// # Returns
/// - `(follower_shard_pda, bump)` — the canonical address and its bump.
pub fn derive_follower_shard_pda(program_id: &Pubkey, thread: &Pubkey, shard: u8) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[FOLLOWER_COUNT_SEED, thread.as_ref(), &[shard]], program_id)
}

/// Deterministically maps a wallet to one of the `N_FOLLOWER_SHARDS` counter
/// shards. Subscribe and unsubscribe must agree so increments/decrements land
/// on the same shard.
///
/// # Parameters
/// - `wallet` — the follower wallet whose shard is chosen.
/// # Returns
/// - The shard index in `0..N_FOLLOWER_SHARDS`.
pub fn follower_shard_index(wallet: &Pubkey) -> u8 {
    wallet.as_ref()[0] % N_FOLLOWER_SHARDS
}
