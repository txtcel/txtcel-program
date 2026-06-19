use borsh::BorshSerialize;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
};

use crate::error::ProtocolError;
use crate::state::*;

/// Transfers admin rights of the program settings to a new authority.
///
/// Steps:
/// 1. Loads the program settings and verifies the caller is the current admin.
/// 2. Updates the `admin` field with the new public key.
/// 3. Serializes the updated settings back to the blockchain.
pub fn process_set_admin(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    new_admin: Pubkey,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let authority = next_account_info(account_info_iter)?;
    let settings_account = next_account_info(account_info_iter)?;

    assert_signer(authority)?;
    assert_writable(settings_account)?;

    let mut settings = load_settings(program_id, settings_account)?;
    if settings.admin != *authority.key {
        return Err(ProtocolError::Unauthorized.into());
    }

    settings.admin = new_admin;
    settings.serialize(&mut &mut settings_account.data.borrow_mut()[..])?;

    Ok(())
}
