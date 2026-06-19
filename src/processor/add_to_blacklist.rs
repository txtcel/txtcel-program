use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
};

use crate::state::*;

/// Blacklists a wallet by creating (or flipping) its per-wallet `AccessEntry` PDA.
///
/// Accounts:
/// 0. `[signer, writable]` authority - Admin of the thread access account (pays rent).
/// 1. `[writable]` access_account - Thread access (ThreadAccess) PDA, used to verify admin.
/// 2. `[writable]` entry_account - AccessEntry PDA derived from [ACL_SEED, seed, wallet].
/// 3. `[]` system_program
pub fn process_add_to_blacklist(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    wallet: Pubkey,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let authority = next_account_info(account_info_iter)?;
    let access_account = next_account_info(account_info_iter)?;
    let entry_account = next_account_info(account_info_iter)?;
    let system_program_account = next_account_info(account_info_iter)?;

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
        &wallet,
        ACCESS_DENIED,
    )?;

    // Blacklisting a wallet that currently holds an allow entry drops it from
    // the whitelist.
    if prev == Some(ACCESS_ALLOWED) {
        adjust_whitelist_count(access_account, -1)?;
    }

    Ok(())
}
