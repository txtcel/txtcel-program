use borsh::BorshSerialize;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
};

use crate::error::ProtocolError;
use crate::state::*;

/// General helper to update a platform-level fee setting.
///
/// Steps:
/// 1. Validates that the requested fee does not exceed the maximum allowed.
/// 2. Loads the program settings account and verifies the authority is the admin.
/// 3. Applies the provided update closure to modify the appropriate fee field.
/// 4. Serializes the updated settings back to the blockchain.
///
/// Individual functions (process_set_base_fee, process_set_author_fee_cut, etc.)
/// simply call this helper with the specific fee field to update.
///
/// Effect:
/// - Changes how future platform operations calculate their fees or cuts.
///
/// # Parameters
/// - `program_id` — this program's address, used for settings PDA/ownership.
/// - `accounts` — `[authority(admin signer), settings]`.
/// - `fee_bps` — new basis-points value, rejected above `MAX_FEE_CUT_BPS`.
/// - `update` — closure that writes `fee_bps` into the chosen settings field.
///
/// # Returns
/// - `Ok(())` once the updated settings are persisted.
/// - `ProtocolError::InvalidFeeBps` if out of range, or admin/PDA errors.
fn update_platform_setting(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    fee_bps: u32,
    update: impl FnOnce(&mut ProgramSettings),
) -> ProgramResult {
    if fee_bps > MAX_FEE_CUT_BPS {
        return Err(ProtocolError::InvalidFeeBps.into());
    }

    let account_info_iter = &mut accounts.iter();
    let authority = next_account_info(account_info_iter)?;
    let settings_account = next_account_info(account_info_iter)?;

    let mut settings = load_admin_settings(program_id, authority, settings_account)?;

    update(&mut settings);

    settings.serialize(&mut &mut settings_account.data.borrow_mut()[..])?;
    Ok(())
}

/// Sets the platform base fee (percentage of rent, in bps).
///
/// # Parameters
/// - `program_id` — this program's address.
/// - `accounts` — `[authority(admin signer), settings]`.
/// - `fee_bps` — new base fee in basis points.
///
/// # Returns
/// - `Ok(())` on success, else the error from `update_platform_setting`.
pub fn process_set_base_fee(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    fee_bps: u32,
) -> ProgramResult {
    update_platform_setting(program_id, accounts, fee_bps, |s| s.base_fee_bps = fee_bps)
}

/// Sets the platform's cut of the per-message author fee (in bps).
///
/// # Parameters
/// - `program_id` — this program's address.
/// - `accounts` — `[authority(admin signer), settings]`.
/// - `fee_bps` — new author-fee cut in basis points.
///
/// # Returns
/// - `Ok(())` on success, else the error from `update_platform_setting`.
pub fn process_set_author_fee_cut(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    fee_bps: u32,
) -> ProgramResult {
    update_platform_setting(program_id, accounts, fee_bps, |s| s.author_fee_cut_bps = fee_bps)
}

/// Sets the platform's cut of the entry fee (in bps).
///
/// # Parameters
/// - `program_id` — this program's address.
/// - `accounts` — `[authority(admin signer), settings]`.
/// - `fee_bps` — new entry-fee cut in basis points.
///
/// # Returns
/// - `Ok(())` on success, else the error from `update_platform_setting`.
pub fn process_set_entry_cut(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    fee_bps: u32,
) -> ProgramResult {
    update_platform_setting(program_id, accounts, fee_bps, |s| s.entry_cut_bps = fee_bps)
}

/// Sets the platform's cut of the like fee (in bps).
///
/// # Parameters
/// - `program_id` — this program's address.
/// - `accounts` — `[authority(admin signer), settings]`.
/// - `fee_bps` — new like-fee cut in basis points.
///
/// # Returns
/// - `Ok(())` on success, else the error from `update_platform_setting`.
pub fn process_set_like_cut(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    fee_bps: u32,
) -> ProgramResult {
    update_platform_setting(program_id, accounts, fee_bps, |s| s.like_cut_bps = fee_bps)
}
