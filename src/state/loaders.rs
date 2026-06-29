//! Account loaders and the higher-level access-control mutations built on them.
//!
//! Loaders deserialize an account, verify its tag (and, where the address is a
//! PDA, its derivation/ownership) so processors get a typed, validated struct
//! instead of raw bytes. The `*_admin_*`/`*_author_*` variants additionally
//! enforce the signer/authority gate for privileged paths. The mutation
//! helpers (set/close access entries, adjust the whitelist counter) bundle the
//! exact create/update/close ordering shared across the ACL instructions.

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::AccountInfo,
    entrypoint::ProgramResult,
    program_error::ProgramError,
    pubkey::Pubkey,
};

use crate::error::ProtocolError;

use super::*;

/// Deserializes a `ThreadAccess` account and verifies its tag. The low-level
/// load with no ownership/authority gate; admin paths use `load_admin_access`.
///
/// # Parameters
/// - `account` — the access account to read.
/// # Returns
/// - `Ok(ThreadAccess)`, or `InvalidAccountData`/`InvalidTag`.
pub fn load_thread_access(account: &AccountInfo) -> Result<ThreadAccess, ProgramError> {
    let mut data = &account.data.borrow()[..];
    let access = ThreadAccess::deserialize(&mut data)
        .map_err(|_| ProtocolError::InvalidAccountData)?;
    if access.tag != TAG_ACCESS {
        return Err(ProtocolError::InvalidTag.into());
    }
    Ok(access)
}

/// Deserializes an `AccessEntry` account and verifies its tag, without checking
/// ownership or the thread/wallet binding; `load_bound_access_entry` adds those.
///
/// # Parameters
/// - `account` — the access-entry account to read.
/// # Returns
/// - `Ok(AccessEntry)`, or `InvalidAccountData`/`InvalidTag`.
pub fn load_access_entry(account: &AccountInfo) -> Result<AccessEntry, ProgramError> {
    let mut data = &account.data.borrow()[..];
    let entry = AccessEntry::deserialize(&mut data)
        .map_err(|_| ProtocolError::InvalidAccountData)?;
    if entry.tag != TAG_ACCESS_ENTRY {
        return Err(ProtocolError::InvalidTag.into());
    }
    Ok(entry)
}

/// Loads an existing `AccessEntry`, asserting program ownership and that it is
/// bound to the given thread/wallet. Centralizes the owner-check + load + bind
/// guard shared by the access-control paths, preserving the exact order:
/// ownership, deserialize/tag, then thread/wallet bind.
///
/// # Parameters
/// - `program_id` — this program; the entry must be owned by it.
/// - `entry_account` — the access-entry account to load.
/// - `thread` — the thread the entry must be bound to.
/// - `wallet` — the wallet the entry must be bound to.
/// # Returns
/// - `Ok(AccessEntry)`, or `AccountOwnerMismatch`/`InvalidTag`/`ThreadMismatch`.
pub fn load_bound_access_entry(
    program_id: &Pubkey,
    entry_account: &AccountInfo,
    thread: &Pubkey,
    wallet: &Pubkey,
) -> Result<AccessEntry, ProgramError> {
    assert_owned_by(entry_account, program_id)?;
    let entry = load_access_entry(entry_account)?;
    if entry.thread != *thread || entry.wallet != *wallet {
        return Err(ProtocolError::ThreadMismatch.into());
    }
    Ok(entry)
}

/// Deserializes an `AllocLikes` account and checks its tag, returning
/// `InvalidAccountData` on a malformed buffer and `InvalidTag` on the wrong tag.
/// Shared by `like_content` (increment) and `close_account` (reset a slot).
///
/// # Parameters
/// - `account` — the likes account to read.
/// # Returns
/// - `Ok(AllocLikes)`, or `InvalidAccountData`/`InvalidTag`.
pub fn load_alloc_likes(account: &AccountInfo) -> Result<AllocLikes, ProgramError> {
    let likes = AllocLikes::try_from_slice(&account.data.borrow())
        .map_err(|_| ProtocolError::InvalidAccountData)?;
    if likes.tag != TAG_LIKES {
        return Err(ProtocolError::InvalidTag.into());
    }
    Ok(likes)
}

/// Deserializes a `FollowRegistry` account and verifies its tag. Ownership and
/// the owner-binding check are left to the caller (the subscribe path).
///
/// # Parameters
/// - `account` — the follow-registry account to read.
/// # Returns
/// - `Ok(FollowRegistry)`, or `InvalidAccountData`/`InvalidTag`.
pub fn load_follow_registry(account: &AccountInfo) -> Result<FollowRegistry, ProgramError> {
    let registry = FollowRegistry::try_from_slice(&account.data.borrow())
        .map_err(|_| ProtocolError::InvalidAccountData)?;
    if registry.tag != TAG_FOLLOW_REGISTRY {
        return Err(ProtocolError::InvalidTag.into());
    }
    Ok(registry)
}

/// Deserializes a `FollowerShard` account and verifies its tag, without the
/// ownership or thread/shard binding checks added by `load_bound_follower_shard`.
///
/// # Parameters
/// - `account` — the follower-shard account to read.
/// # Returns
/// - `Ok(FollowerShard)`, or `InvalidAccountData`/`InvalidTag`.
pub fn load_follower_shard(account: &AccountInfo) -> Result<FollowerShard, ProgramError> {
    let shard = FollowerShard::try_from_slice(&account.data.borrow())
        .map_err(|_| ProtocolError::InvalidAccountData)?;
    if shard.tag != TAG_FOLLOWER_SHARD {
        return Err(ProtocolError::InvalidTag.into());
    }
    Ok(shard)
}

/// Loads an existing `FollowerShard`, asserting program ownership and that it is
/// bound to the given thread and shard index. Shared by subscribe (increment)
/// and unsubscribe (decrement); preserves the order: ownership, deserialize/tag,
/// then thread/shard bind.
///
/// # Parameters
/// - `program_id` — this program; the shard must be owned by it.
/// - `shard_account` — the follower-shard account to load.
/// - `thread` — the channel the shard must be bound to.
/// - `shard_idx` — the shard index the account must match.
/// # Returns
/// - `Ok(FollowerShard)`, or `AccountOwnerMismatch`/`InvalidTag`/`ThreadMismatch`.
pub fn load_bound_follower_shard(
    program_id: &Pubkey,
    shard_account: &AccountInfo,
    thread: &Pubkey,
    shard_idx: u8,
) -> Result<FollowerShard, ProgramError> {
    assert_owned_by(shard_account, program_id)?;
    let shard = load_follower_shard(shard_account)?;
    if shard.thread != *thread || shard.shard != shard_idx {
        return Err(ProtocolError::ThreadMismatch.into());
    }
    Ok(shard)
}

/// Creates the `AccessEntry` PDA for `wallet` if missing, otherwise flips its
/// status. `payer` funds creation. Used by whitelist/blacklist management.
///
/// # Parameters
/// - `program_id` — the program that owns the entry PDA.
/// - `payer` — funds entry creation on first use.
/// - `entry_account` — the `AccessEntry` PDA being created or updated.
/// - `system_program_account` — System Program for creation CPIs.
/// - `thread` — the thread the entry is bound to (part of the PDA seeds).
/// - `wallet` — the wallet the entry is for (part of the PDA seeds).
/// - `status` — the new membership status to store.
/// # Returns
/// - `Ok(None)` when the entry was newly created, or `Ok(Some(prev))` with the
///   status it held before this call; callers use it to keep the whitelist
///   counter accurate across status transitions. Errors propagate from PDA
///   validation/creation/serialization.
pub fn set_access_entry_status<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    entry_account: &AccountInfo<'a>,
    system_program_account: &AccountInfo<'a>,
    thread: &Pubkey,
    wallet: &Pubkey,
    status: u8,
) -> Result<Option<u8>, ProgramError> {
    let (expected_entry, entry_bump) = derive_access_entry_pda(program_id, thread, wallet);
    assert_pda(entry_account, &expected_entry)?;

    if is_uninitialized(entry_account) {
        let size = AccessEntry::size();

        create_pda_account(
            program_id,
            payer,
            entry_account,
            system_program_account,
            size,
            &[ACL_SEED, thread.as_ref(), wallet.as_ref(), &[entry_bump]],
        )?;

        let entry = AccessEntry {
            tag: TAG_ACCESS_ENTRY,
            thread: *thread,
            wallet: *wallet,
            status,
        };
        entry.serialize(&mut &mut entry_account.data.borrow_mut()[..])?;

        Ok(None)
    } else {
        let mut entry = load_bound_access_entry(program_id, entry_account, thread, wallet)?;
        let prev = entry.status;
        entry.status = status;
        entry.serialize(&mut &mut entry_account.data.borrow_mut()[..])?;

        Ok(Some(prev))
    }
}

/// Applies a signed delta to the thread's whitelist member counter, saturating
/// at zero, and persists it. `access_account` must be the program-owned,
/// writable `ThreadAccess` PDA (callers obtain it via `load_admin_access` or an
/// equivalent ownership check).
///
/// # Parameters
/// - `access_account` — the program-owned, writable `ThreadAccess` PDA.
/// - `delta` — signed change to apply to the whitelist member count.
/// # Returns
/// - `Ok(())` once the counter is updated and persisted, else a load/serialize error.
pub fn adjust_whitelist_count(access_account: &AccountInfo, delta: i64) -> ProgramResult {
    let mut access = load_thread_access(access_account)?;
    access.whitelist_count = (access.whitelist_count as i64)
        .saturating_add(delta)
        .max(0) as u32;
    access.serialize(&mut &mut access_account.data.borrow_mut()[..])?;
    Ok(())
}

/// Closes an `AccessEntry`, returning its rent to `recipient`. Requires the
/// entry to currently hold `expected_status` (so "remove from whitelist" only
/// removes allow-entries and vice-versa).
///
/// # Parameters
/// - `program_id` — the program that owns the entry PDA.
/// - `recipient` — receives the closed entry's refunded rent.
/// - `entry_account` — the `AccessEntry` PDA being closed.
/// - `_system_program_account` — unused; kept for call-site signature symmetry.
/// - `thread` — the thread the entry must be bound to.
/// - `wallet` — the wallet the entry must be bound to.
/// - `expected_status` — the status the entry must currently hold.
/// # Returns
/// - `Ok(())` once closed, or `AccessListMissing` if absent or holding a
///   different status, plus any PDA/bind validation error.
pub fn close_access_entry<'a>(
    program_id: &Pubkey,
    recipient: &AccountInfo<'a>,
    entry_account: &AccountInfo<'a>,
    _system_program_account: &AccountInfo<'a>,
    thread: &Pubkey,
    wallet: &Pubkey,
    expected_status: u8,
) -> ProgramResult {
    let (expected_entry, _) = derive_access_entry_pda(program_id, thread, wallet);
    assert_pda(entry_account, &expected_entry)?;

    if is_uninitialized(entry_account) {
        return Err(ProtocolError::AccessListMissing.into());
    }

    let entry = load_bound_access_entry(program_id, entry_account, thread, wallet)?;
    if entry.status != expected_status {
        return Err(ProtocolError::AccessListMissing.into());
    }

    close_program_account(entry_account, recipient)?;

    Ok(())
}

/// Sets `wallet`'s `AccessEntry` to `status` (creating the PDA when missing) and
/// keeps the thread's whitelist counter consistent. The counter tracks
/// `ACCESS_ALLOWED` members, so the delta is derived purely from the
/// previous-to-target transition (`+1` entering the allow set, `-1` leaving it,
/// `0` otherwise) instead of per-status ad-hoc rules. Shared by the three
/// `add_to_*` ACL setters; preserves their order: system-program/writable
/// checks, admin load, status write, then counter adjustment.
///
/// # Parameters
/// - `program_id` — this program's id.
/// - `authority` — the ACL admin signer (also funds entry creation).
/// - `access_account` — the thread's `ThreadAccess` PDA (admin-gated, writable).
/// - `entry_account` — the target wallet's `AccessEntry` PDA.
/// - `system_program_account` — System Program for entry creation.
/// - `wallet` — the wallet whose membership is being set.
/// - `status` — the new membership status to apply.
/// # Returns
/// - `Ok(())` once the status is written and the whitelist counter reconciled,
///   else the first failing step's error.
pub fn apply_access_entry_status<'a>(
    program_id: &Pubkey,
    authority: &AccountInfo<'a>,
    access_account: &AccountInfo<'a>,
    entry_account: &AccountInfo<'a>,
    system_program_account: &AccountInfo<'a>,
    wallet: &Pubkey,
    status: u8,
) -> ProgramResult {
    assert_system_program(system_program_account)?;
    assert_writable(authority)?;
    assert_writable(entry_account)?;

    let access = load_admin_access(program_id, authority, access_account)?;
    let thread = access.thread;

    let prev = set_access_entry_status(
        program_id,
        authority,
        entry_account,
        system_program_account,
        &thread,
        wallet,
        status,
    )?;

    let was_allowed = prev == Some(ACCESS_ALLOWED);
    let now_allowed = status == ACCESS_ALLOWED;
    let delta = i64::from(now_allowed) - i64::from(was_allowed);
    if delta != 0 {
        adjust_whitelist_count(access_account, delta)?;
    }

    Ok(())
}

/// Closes `wallet`'s `AccessEntry`, requiring it to currently hold
/// `expected_status`, and decrements the whitelist counter only when an allow
/// entry was removed (the counter tracks `ACCESS_ALLOWED` members, so removing a
/// deny or fee-exempt entry leaves it unchanged). Shared by the three
/// `remove_from_*` ACL setters; preserves their order: writable checks, admin
/// load, entry close, then counter adjustment.
///
/// # Parameters
/// - `program_id` — this program's id.
/// - `authority` — the ACL admin signer (also the rent-refund recipient).
/// - `access_account` — the thread's `ThreadAccess` PDA (admin-gated, writable).
/// - `entry_account` — the target wallet's `AccessEntry` PDA being closed.
/// - `system_program_account` — System Program (passed through to the close).
/// - `wallet` — the wallet whose membership is being removed.
/// - `expected_status` — the status the entry must currently hold to be removed.
/// # Returns
/// - `Ok(())` once the entry is closed and the counter reconciled, else the
///   first failing step's error.
pub fn remove_access_entry<'a>(
    program_id: &Pubkey,
    authority: &AccountInfo<'a>,
    access_account: &AccountInfo<'a>,
    entry_account: &AccountInfo<'a>,
    system_program_account: &AccountInfo<'a>,
    wallet: &Pubkey,
    expected_status: u8,
) -> ProgramResult {
    assert_writable(authority)?;
    assert_writable(entry_account)?;

    let access = load_admin_access(program_id, authority, access_account)?;
    let thread = access.thread;

    close_access_entry(
        program_id,
        authority,
        entry_account,
        system_program_account,
        &thread,
        wallet,
        expected_status,
    )?;

    if expected_status == ACCESS_ALLOWED {
        adjust_whitelist_count(access_account, -1)?;
    }

    Ok(())
}

/// Loads a `ThreadNode`. The thread is a plain program-owned account whose
/// address is its own identity (a full pubkey, not a PDA), so validation is
/// limited to ownership + tag. Child PDAs are bound to this account's key.
///
/// # Parameters
/// - `program_id` — this program; the thread must be owned by it.
/// - `account` — the thread account to load.
/// # Returns
/// - `Ok(ThreadNode)`, or `AccountOwnerMismatch`/`InvalidAccountData`/`InvalidTag`.
pub fn load_thread(program_id: &Pubkey, account: &AccountInfo) -> Result<ThreadNode, ProgramError> {
    assert_owned_by(account, program_id)?;

    let thread = ThreadNode::try_from_slice(&account.data.borrow())
        .map_err(|_| ProtocolError::InvalidAccountData)?;

    if thread.tag != TAG_THREAD {
        return Err(ProtocolError::InvalidTag.into());
    }

    Ok(thread)
}

/// Loads a `ThreadNode` for an author-only mutation: asserts the authority
/// signed, the thread account is writable, then loads it and verifies the
/// authority is the thread's author. Mirrors `load_admin_access` for the
/// thread-owner paths (`set_message_fee`, `set_like_fee`) and preserves their
/// order: signer, writable, load (owner/tag), then author check.
///
/// # Parameters
/// - `program_id` — this program; the thread must be owned by it.
/// - `authority` — the signer that must be the thread's author.
/// - `thread_account` — the thread account (must be writable).
/// # Returns
/// - `Ok(ThreadNode)`, or `MissingSigner`/`NotWritable`/load error/`Unauthorized`.
pub fn load_author_thread(
    program_id: &Pubkey,
    authority: &AccountInfo,
    thread_account: &AccountInfo,
) -> Result<ThreadNode, ProgramError> {
    assert_signer(authority)?;
    assert_writable(thread_account)?;

    let thread = load_thread(program_id, thread_account)?;

    if *authority.key != thread.author {
        return Err(ProtocolError::Unauthorized.into());
    }

    Ok(thread)
}

/// Loads an `AllocNode`, asserting program ownership, tag, and that the account
/// address is the canonical alloc PDA for its own `(thread, alloc_seq)` — so a
/// node cannot masquerade as a different position in the alloc chain.
///
/// # Parameters
/// - `program_id` — this program; the alloc must be owned by it.
/// - `account` — the alloc account to load.
/// # Returns
/// - `Ok(AllocNode)`, or `AccountOwnerMismatch`/`InvalidAccountData`/`InvalidTag`/`InvalidPda`.
pub fn load_alloc(program_id: &Pubkey, account: &AccountInfo) -> Result<AllocNode, ProgramError> {
    assert_owned_by(account, program_id)?;

    let alloc = AllocNode::try_from_slice(&account.data.borrow())
        .map_err(|_| ProtocolError::InvalidAccountData)?;

    if alloc.tag != TAG_ALLOC {
        return Err(ProtocolError::InvalidTag.into());
    }

    let (expected, _) = derive_alloc_pda(program_id, &alloc.thread, alloc.alloc_seq);

    assert_pda(account, &expected)?;

    Ok(alloc)
}

/// Loads a `ThreadAccess` for an ACL-admin mutation: asserts the authority
/// signed and the account is writable + program-owned, then verifies the
/// authority is the thread's ACL admin and the account is the canonical access
/// PDA. The authority gate for whitelist/blacklist management.
///
/// # Parameters
/// - `program_id` — this program; the access account must be owned by it.
/// - `authority` — the signer that must be the thread's ACL admin.
/// - `access_account` — the `ThreadAccess` PDA (must be writable).
/// # Returns
/// - `Ok(ThreadAccess)`, or `MissingSigner`/`NotWritable`/`AccountOwnerMismatch`/
///   `Unauthorized`/`InvalidPda`.
pub fn load_admin_access(
    program_id: &Pubkey,
    authority: &AccountInfo,
    access_account: &AccountInfo,
) -> Result<ThreadAccess, ProgramError> {
    assert_signer(authority)?;
    assert_writable(access_account)?;
    assert_owned_by(access_account, program_id)?;

    let access = load_thread_access(access_account)?;

    if access.admin != *authority.key {
        return Err(ProtocolError::Unauthorized.into());
    }

    let (expected, _) = derive_access_pda(program_id, &access.thread);

    assert_pda(access_account, &expected)?;

    Ok(access)
}

/// Loads the singleton `ProgramSettings`, asserting it is the canonical settings
/// PDA, program-owned, and correctly tagged. The read path used wherever global
/// fee/admin config is needed; `load_admin_settings` adds the admin gate.
///
/// # Parameters
/// - `program_id` — this program; the settings account must be owned by it.
/// - `settings_account` — the singleton `ProgramSettings` PDA.
/// # Returns
/// - `Ok(ProgramSettings)`, or `InvalidPda`/`AccountOwnerMismatch`/
///   `InvalidAccountData`/`InvalidTag`.
pub fn load_settings(
    program_id: &Pubkey,
    settings_account: &AccountInfo,
) -> Result<ProgramSettings, ProgramError> {
    let (expected_settings, _) = derive_settings_pda(program_id);

    assert_pda(settings_account, &expected_settings)?;

    if settings_account.owner != program_id {
        return Err(ProtocolError::AccountOwnerMismatch.into());
    }

    let settings = ProgramSettings::try_from_slice(&settings_account.data.borrow())
        .map_err(|_| ProtocolError::InvalidAccountData)?;

    if settings.tag != TAG_SETTINGS {
        return Err(ProtocolError::InvalidTag.into());
    }

    Ok(settings)
}

/// Loads `ProgramSettings` for an admin-only mutation: asserts the authority
/// signed, the settings account is writable, then loads it (PDA/owner/tag) and
/// verifies the authority is the current admin. Mirrors `load_admin_access` for
/// the global-settings paths (`set_admin`, `set_treasury`, the platform-fee
/// setters) and preserves their order: signer, writable, load, then admin check.
///
/// # Parameters
/// - `program_id` — this program's id.
/// - `authority` — the signer that must be the current admin.
/// - `settings_account` — the `ProgramSettings` PDA (must be writable).
/// # Returns
/// - `Ok(ProgramSettings)`, or `MissingSigner`/`NotWritable`/load error/`Unauthorized`.
pub fn load_admin_settings(
    program_id: &Pubkey,
    authority: &AccountInfo,
    settings_account: &AccountInfo,
) -> Result<ProgramSettings, ProgramError> {
    assert_signer(authority)?;
    assert_writable(settings_account)?;

    let settings = load_settings(program_id, settings_account)?;

    if settings.admin != *authority.key {
        return Err(ProtocolError::Unauthorized.into());
    }

    Ok(settings)
}
