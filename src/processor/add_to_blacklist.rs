use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
};

use crate::state::*;

/// Blacklists a wallet by creating (or flipping) its per-wallet `AccessEntry` PDA.
///
/// Blacklisting a wallet that currently holds an allow entry drops it from the
/// whitelist.
///
/// Accounts:
/// 0. `[signer, writable]` authority - Admin of the thread access account (pays rent).
/// 1. `[writable]` access_account - Thread access (ThreadAccess) PDA, used to verify admin.
/// 2. `[writable]` entry_account - AccessEntry PDA derived from [ACL_SEED, seed, wallet].
/// 3. `[]` system_program
///
/// # Parameters
/// - `program_id` — this program's address, used for PDA derivation/ownership.
/// - `accounts` — the account list described above, in order.
/// - `wallet` — wallet being blacklisted (deny entry).
///
/// # Returns
/// - `Ok(())` once the wallet holds an `ACCESS_DENIED` entry.
/// - Admin/PDA/account-creation errors from `apply_access_entry_status`.
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

    apply_access_entry_status(
        program_id,
        authority,
        access_account,
        entry_account,
        system_program_account,
        &wallet,
        ACCESS_DENIED,
    )
}
