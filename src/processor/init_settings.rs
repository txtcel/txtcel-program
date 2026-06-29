use borsh::BorshSerialize;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
};

use crate::state::*;

/// Initializes the program settings account with default parameters and a designated treasury.
///
/// Steps:
/// 1. Validates required accounts: authority (signer), settings PDA, program data account (upgrade authority), and system program.
/// 2. Verifies that the settings account matches the expected PDA and is uninitialized.
/// 3. Calculates the rent-exempt lamports for the settings account.
/// 4. Creates the settings account on-chain using `invoke_signed`.
/// 5. Initializes the ProgramSettings struct with:
///    - Admin set to the authority.
///    - Treasury address.
///    - Default fee percentages (base_fee, author_fee_cut, entry_cut, like_cut) all set to 1000 basis points (10%).
/// 6. Serializes the settings data into the account.
///
/// After execution:
/// - A rent-exempt settings account exists on-chain, ready to govern fees and treasury for threads and allocations.
///
/// # Parameters
/// - `program_id` — this program's address, used for settings PDA/ownership.
/// - `accounts` — `[authority(upgrade-authority signer), settings, programdata,
///   system]`.
/// - `treasury` — wallet that will receive swept platform revenue.
///
/// # Returns
/// - `Ok(())` once the settings account is created and initialized.
/// - PDA/authority/`assert_uninitialized` errors, or account-creation failures.
pub fn process_init_settings(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    treasury: Pubkey,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let authority = next_account_info(account_info_iter)?;
    let settings_account = next_account_info(account_info_iter)?;
    let programdata_account = next_account_info(account_info_iter)?;
    let system_program_account = next_account_info(account_info_iter)?;

    assert_signer(authority)?;
    assert_writable(settings_account)?;
    assert_upgrade_authority(program_id, programdata_account, authority)?;
    assert_system_program(system_program_account)?;

    let (expected_settings, settings_bump) = derive_settings_pda(program_id);
    assert_pda(settings_account, &expected_settings)?;

    assert_uninitialized(settings_account)?;

    let size = ProgramSettings::size();

    create_pda_account(
        program_id,
        authority,
        settings_account,
        system_program_account,
        size,
        &[SETTINGS_SEED, &[settings_bump]],
    )?;

    let settings = ProgramSettings {
        tag: TAG_SETTINGS,
        admin: *authority.key,
        treasury,
        base_fee_bps: 1000,
        author_fee_cut_bps: 1000,
        entry_cut_bps: 1000,
        like_cut_bps: 1000,
    };
    settings.serialize(&mut &mut settings_account.data.borrow_mut()[..])?;

    Ok(())
}
