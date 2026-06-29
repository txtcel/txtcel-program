use borsh::BorshSerialize;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
    sysvar::{rent::Rent, Sysvar},
};

use crate::error::ProtocolError;
use crate::state::*;

/// Initializes a thread-specific access control account, allowing the author to manage whitelists and blacklists.
///
/// Steps:
/// 1. Validates required accounts: authority (signer), thread, access PDA, treasury shard, and system program.
/// 2. Ensures the authority is the thread author and the access account matches the expected PDA and is uninitialized.
/// 3. Checks that the requested capacity is within allowed bounds.
/// 4. Ensures the treasury shard is initialized and ready to collect fees.
/// 5. Calculates the rent-exempt lamports for the access account and collects them from the authority into the treasury shard.
/// 6. Creates the access account on-chain using `invoke_signed`.
/// 7. Initializes a `ThreadAccess` struct with:
///    - Tag identifying it as an access account.
///    - Seed matching the thread.
///    - Enabled status (open/closed).
///    - Admin set to the authority.
///    - Entry fee set to zero initially.
/// 8. Serializes the `ThreadAccess` struct into the access account.
///
/// Per-wallet allow/deny membership is stored in separate `AccessEntry` PDAs
/// (see `set_access_entry_status`), not inside this account.
///
/// After execution:
/// - A rent-exempt access control account exists for the thread, ready to enforce whitelist/blacklist rules and manage entry fees.
///
/// # Parameters
/// - `program_id` — this program's address, used for PDA derivation/ownership.
/// - `accounts` — `[authority(thread author signer), thread, access,
///   treasury_shard, system]`.
/// - `enabled` — whether gating starts enabled for the thread.
/// - `treasury_shard_idx` — treasury shard collecting the rent fee.
///
/// # Returns
/// - `Ok(())` once the access account is created and initialized.
/// - `ProtocolError::Unauthorized`, PDA/`assert_uninitialized`, or fee errors.
pub fn process_init_thread_access(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    enabled: bool,
    treasury_shard_idx: u16,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let authority = next_account_info(account_info_iter)?;
    let thread_account = next_account_info(account_info_iter)?;
    let access_account = next_account_info(account_info_iter)?;
    let treasury_shard = next_account_info(account_info_iter)?;
    let system_program_account = next_account_info(account_info_iter)?;

    assert_signer(authority)?;
    assert_writable(access_account)?;
    assert_writable(treasury_shard)?;
    assert_system_program(system_program_account)?;

    let thread = load_thread(program_id, thread_account)?;
    let thread_key = *thread_account.key;

    if thread.author != *authority.key {
        return Err(ProtocolError::Unauthorized.into());
    }

    let (expected_access, access_bump) = derive_access_pda(program_id, &thread_key);

    assert_pda(access_account, &expected_access)?;

    assert_uninitialized(access_account)?;

    let treasury_shard_bump = validate_treasury_shard(program_id, treasury_shard, treasury_shard_idx)?;

    ensure_shard_initialized(
        program_id,
        authority,
        treasury_shard,
        system_program_account,
        &[TREASURY_SHARD_SEED, &treasury_shard_idx.to_le_bytes()],
        treasury_shard_bump,
    )?;

    let rent = Rent::get()?;
    let size = ThreadAccess::size();
    let lamports = rent.minimum_balance(size);

    collect_fee_to_shard(lamports, authority, treasury_shard, system_program_account)?;

    create_pda_account(
        program_id,
        authority,
        access_account,
        system_program_account,
        size,
        &[ACCESS_SEED, thread_key.as_ref(), &[access_bump]],
    )?;

    let access = ThreadAccess {
        tag: TAG_ACCESS,
        thread: thread_key,
        enabled,
        admin: *authority.key,
        entry_fee: 0,
        whitelist_count: 0,
    };

    access.serialize(&mut &mut access_account.data.borrow_mut()[..])?;

    Ok(())
}
