use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
};

use crate::state::*;

/// Removes a wallet from the blacklist by closing its deny `AccessEntry` PDA.
///
/// Accounts:
/// 0. `[signer, writable]` authority - Admin of the thread access account (receives rent).
/// 1. `[writable]` access_account - Thread access (ThreadAccess) PDA, used to verify admin.
/// 2. `[writable]` entry_account - AccessEntry PDA derived from [ACL_SEED, seed, wallet].
/// 3. `[]` system_program
///
/// # Parameters
/// - `program_id` — this program's address, used for PDA derivation/ownership.
/// - `accounts` — the account list described above, in order.
/// - `wallet` — wallet whose deny entry is removed.
///
/// # Returns
/// - `Ok(())` once the wallet's `ACCESS_DENIED` entry is closed.
/// - Admin/PDA errors, or if no matching deny entry exists.
pub fn process_remove_from_blacklist(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    wallet: Pubkey,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let authority = next_account_info(account_info_iter)?;
    let access_account = next_account_info(account_info_iter)?;
    let entry_account = next_account_info(account_info_iter)?;
    let system_program_account = next_account_info(account_info_iter)?;

    remove_access_entry(
        program_id,
        authority,
        access_account,
        entry_account,
        system_program_account,
        &wallet,
        ACCESS_DENIED,
    )
}
