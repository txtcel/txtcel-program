use borsh::BorshSerialize;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
};

use crate::state::*;

/// Updates the access control status of a thread.
///
/// Steps:
/// 1. Loads the admin access account associated with the thread.
/// 2. Sets the `enabled` flag according to the provided value.
/// 3. Serializes the updated access account back to the blockchain.
///
/// Effect:
/// - Enables or disables user access requests and entry checks for this thread.
///
/// # Parameters
/// - `program_id` — this program's address, used for access PDA/ownership.
/// - `accounts` — `[authority(access admin signer), access]`.
/// - `enabled` — new gating state for the thread.
///
/// # Returns
/// - `Ok(())` once the new state is persisted.
/// - Admin/PDA validation errors from `load_admin_access`.
pub fn process_set_thread_access(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    enabled: bool,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let authority = next_account_info(account_info_iter)?;
    let access_account = next_account_info(account_info_iter)?;

    let mut access = load_admin_access(program_id, authority, access_account)?;

    access.enabled = enabled;
    access.serialize(&mut &mut access_account.data.borrow_mut()[..])?;

    Ok(())
}
