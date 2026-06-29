use borsh::BorshSerialize;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
};

use crate::state::*;

/// Updates the treasury account in the program settings.
///
/// Steps:
/// 1. Loads the program settings account.
/// 2. Verifies that the caller is the admin.
/// 3. Updates the `treasury` field with the new public key.
/// 4. Serializes the updated settings back to the blockchain.
///
/// Effect:
/// - Changes the destination account for collected fees and platform revenue.
///
/// # Parameters
/// - `program_id` — this program's address, used for settings PDA/ownership.
/// - `accounts` — `[authority(admin signer), settings]`.
/// - `treasury` — new destination wallet for platform revenue.
///
/// # Returns
/// - `Ok(())` once the new treasury is persisted.
/// - Admin/PDA validation errors from `load_admin_settings`.
pub fn process_set_treasury(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    treasury: Pubkey,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let authority = next_account_info(account_info_iter)?;
    let settings_account = next_account_info(account_info_iter)?;

    let mut settings = load_admin_settings(program_id, authority, settings_account)?;
    settings.treasury = treasury;
    settings.serialize(&mut &mut settings_account.data.borrow_mut()[..])?;

    Ok(())
}
