use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
};

use crate::error::ProtocolError;
use crate::state::*;

/// Allows a user to gain access to a gated thread by paying the entry fee.
///
/// Creates the caller's own allow `AccessEntry` PDA after splitting the entry
/// fee between the author and treasury.
///
/// Accounts:
/// 0. `[signer, writable]` payer
/// 1. `[writable]` access_account - Thread access (ThreadAccess) PDA (whitelist counter is bumped).
/// 2. `[writable]` entry_account - AccessEntry PDA for the payer.
/// 3. `[]` thread_account
/// 4. `[]` settings_account
/// 5. `[writable]` treasury_shard
/// 6. `[writable]` author_fee_shard
/// 7. `[]` system_program
pub fn process_request_access(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    treasury_shard_idx: u16,
    author_fee_shard_idx: u8,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let payer = next_account_info(account_info_iter)?;
    let access_account = next_account_info(account_info_iter)?;
    let entry_account = next_account_info(account_info_iter)?;
    let thread_account = next_account_info(account_info_iter)?;
    let settings_account = next_account_info(account_info_iter)?;
    let treasury_shard = next_account_info(account_info_iter)?;
    let author_fee_shard = next_account_info(account_info_iter)?;
    let system_program_account = next_account_info(account_info_iter)?;

    assert_signer(payer)?;
    assert_writable(payer)?;
    assert_writable(access_account)?;
    assert_writable(entry_account)?;
    assert_writable(treasury_shard)?;
    assert_writable(author_fee_shard)?;
    assert_system_program(system_program_account)?;
    assert_owned_by(access_account, program_id)?;

    let thread_key = *thread_account.key;

    let (expected_access, _) = derive_access_pda(program_id, &thread_key);
    if *access_account.key != expected_access {
        return Err(ProtocolError::InvalidPda.into());
    }

    let access = load_thread_access(access_account)?;
    if access.thread != thread_key {
        return Err(ProtocolError::ThreadMismatch.into());
    }
    if access.entry_fee == 0 {
        return Err(ProtocolError::ZeroEntryFee.into());
    }

    // Validates the thread account (owner + tag); its key is the channel id.
    let _thread = load_thread(program_id, thread_account)?;

    let (expected_entry, _) = derive_access_entry_pda(program_id, &thread_key, payer.key);
    if *entry_account.key != expected_entry {
        return Err(ProtocolError::InvalidPda.into());
    }

    // Reject blacklisted wallets and already-granted duplicates before charging.
    if !is_uninitialized(entry_account) {
        assert_owned_by(entry_account, program_id)?;
        let entry = load_access_entry(entry_account)?;
        if entry.thread != thread_key || entry.wallet != *payer.key {
            return Err(ProtocolError::ThreadMismatch.into());
        }
        if entry.status == ACCESS_DENIED {
            return Err(ProtocolError::AccessDenied.into());
        }
        return Err(ProtocolError::AccessListDuplicate.into());
    }

    let settings = load_settings(program_id, settings_account)?;

    let treasury_shard_bump = validate_treasury_shard(program_id, treasury_shard, treasury_shard_idx)?;
    let author_fee_shard_bump = validate_author_fee_shard(program_id, &thread_key, author_fee_shard, author_fee_shard_idx)?;

    ensure_shard_initialized(
        program_id,
        payer,
        treasury_shard,
        system_program_account,
        &[TREASURY_SHARD_SEED, &treasury_shard_idx.to_le_bytes()],
        treasury_shard_bump,
    )?;

    ensure_shard_initialized(
        program_id,
        payer,
        author_fee_shard,
        system_program_account,
        &[AUTHOR_FEE_SEED, thread_key.as_ref(), &[author_fee_shard_idx]],
        author_fee_shard_bump,
    )?;

    transfer_fee_split(
        access.entry_fee,
        settings.entry_cut_bps,
        payer,
        author_fee_shard,
        treasury_shard,
        system_program_account,
    )?;

    // A successful request only reaches here for a brand-new entry (existing
    // allow/deny entries are rejected above), so the payer always joins the
    // whitelist as a fresh member.
    set_access_entry_status(
        program_id,
        payer,
        entry_account,
        system_program_account,
        &thread_key,
        payer.key,
        ACCESS_ALLOWED,
    )?;

    adjust_whitelist_count(access_account, 1)?;

    Ok(())
}
