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
///
/// Notes:
/// - The thread account is validated (owner + tag); its key is the channel id.
/// - Blacklisted wallets and already-granted duplicates are rejected before any
///   fee is charged.
/// - A successful request only reaches the entry-creation step for a brand-new
///   entry (existing allow/deny entries are rejected above), so the payer always
///   joins the whitelist as a fresh member.
///
/// # Parameters
/// - `program_id` — this program's address, used for PDA derivation/ownership.
/// - `accounts` — the account list described above, in order.
/// - `treasury_shard_idx` — treasury shard collecting the platform entry cut.
/// - `author_fee_shard_idx` — shard collecting the author's entry share.
///
/// # Returns
/// - `Ok(())` once the entry fee is split and the allow entry created.
/// - `ProtocolError::ThreadMismatch`/`ZeroEntryFee`/`AccessDenied`/
///   `AccessListDuplicate`, or PDA/transfer errors.
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
    assert_fee_shard_accounts(treasury_shard, author_fee_shard, system_program_account)?;
    assert_owned_by(access_account, program_id)?;

    let thread_key = *thread_account.key;

    let (expected_access, _) = derive_access_pda(program_id, &thread_key);
    assert_pda(access_account, &expected_access)?;

    let access = load_thread_access(access_account)?;
    if access.thread != thread_key {
        return Err(ProtocolError::ThreadMismatch.into());
    }
    if access.entry_fee == 0 {
        return Err(ProtocolError::ZeroEntryFee.into());
    }

    let _thread = load_thread(program_id, thread_account)?;

    let (expected_entry, _) = derive_access_entry_pda(program_id, &thread_key, payer.key);
    assert_pda(entry_account, &expected_entry)?;

    if !is_uninitialized(entry_account) {
        let entry = load_bound_access_entry(program_id, entry_account, &thread_key, payer.key)?;
        if entry.status == ACCESS_DENIED {
            return Err(ProtocolError::AccessDenied.into());
        }
        return Err(ProtocolError::AccessListDuplicate.into());
    }

    let settings = load_settings(program_id, settings_account)?;

    prepare_fee_shards(
        program_id,
        payer,
        treasury_shard,
        author_fee_shard,
        system_program_account,
        &thread_key,
        treasury_shard_idx,
        author_fee_shard_idx,
    )?;

    transfer_fee_split(
        access.entry_fee,
        settings.entry_cut_bps,
        payer,
        author_fee_shard,
        treasury_shard,
        system_program_account,
    )?;

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
