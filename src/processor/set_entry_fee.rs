use borsh::BorshSerialize;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
};

use crate::state::*;

/// Sets or updates the entry fee for a thread's access control.
///
/// Steps:
/// 1. Loads the admin access account associated with the thread.
/// 2. Updates the `entry_fee` field with the new fee value provided.
/// 3. Serializes the updated access account data back to the blockchain.
///
/// After execution:
/// - Users requesting access to the thread will be required to pay the new fee.
///
/// # Parameters
/// - `program_id` — this program's address, used for access PDA/ownership.
/// - `accounts` — `[authority(access admin signer), access]`.
/// - `fee` — new entry fee in lamports.
///
/// # Returns
/// - `Ok(())` once the new entry fee is persisted.
/// - Admin/PDA validation errors from `load_admin_access`.
pub fn process_set_entry_fee(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    fee: u64,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let authority = next_account_info(account_info_iter)?;
    let access_account = next_account_info(account_info_iter)?;

    let mut access = load_admin_access(program_id, authority, access_account)?;

    access.entry_fee = fee;
    access.serialize(&mut &mut access_account.data.borrow_mut()[..])?;

    Ok(())
}
