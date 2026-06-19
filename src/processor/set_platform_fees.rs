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

    assert_signer(authority)?;
    assert_writable(settings_account)?;

    let mut settings = load_settings(program_id, settings_account)?;

    if settings.admin != *authority.key {
        return Err(ProtocolError::Unauthorized.into());
    }

    update(&mut settings);

    settings.serialize(&mut &mut settings_account.data.borrow_mut()[..])?;
    Ok(())
}

pub fn process_set_base_fee(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    fee_bps: u32,
) -> ProgramResult {
    update_platform_setting(program_id, accounts, fee_bps, |s| s.base_fee_bps = fee_bps)
}

pub fn process_set_author_fee_cut(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    fee_bps: u32,
) -> ProgramResult {
    update_platform_setting(program_id, accounts, fee_bps, |s| s.author_fee_cut_bps = fee_bps)
}

pub fn process_set_entry_cut(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    fee_bps: u32,
) -> ProgramResult {
    update_platform_setting(program_id, accounts, fee_bps, |s| s.entry_cut_bps = fee_bps)
}

pub fn process_set_like_cut(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    fee_bps: u32,
) -> ProgramResult {
    update_platform_setting(program_id, accounts, fee_bps, |s| s.like_cut_bps = fee_bps)
}
