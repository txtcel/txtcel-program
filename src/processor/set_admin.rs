use borsh::BorshSerialize;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
};

use crate::state::*;

/// Transfers admin rights of the program settings to a new authority.
///
/// Steps:
/// 1. Loads the program settings and verifies the caller is the current admin.
/// 2. Updates the `admin` field with the new public key.
/// 3. Serializes the updated settings back to the blockchain.
///
/// # Parameters
/// - `program_id` — this program's address, used for settings PDA/ownership.
/// - `accounts` — `[authority(current admin signer), settings]`.
/// - `new_admin` — wallet to become the new program admin.
///
/// # Returns
/// - `Ok(())` once the new admin is persisted.
/// - Admin/PDA validation errors from `load_admin_settings`.
pub fn process_set_admin(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    new_admin: Pubkey,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let authority = next_account_info(account_info_iter)?;
    let settings_account = next_account_info(account_info_iter)?;

    let mut settings = load_admin_settings(program_id, authority, settings_account)?;

    settings.admin = new_admin;
    settings.serialize(&mut &mut settings_account.data.borrow_mut()[..])?;

    Ok(())
}
