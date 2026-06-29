use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
    sysvar::{rent::Rent, Sysvar},
};

use crate::error::ProtocolError;
use crate::state::*;

/// Sweeps accumulated treasury funds from all treasury shard accounts into the treasury wallet.
///
/// Steps:
/// 1. Confirms that the provided treasury wallet matches the treasury address in the program settings.
/// 2. Iterates over the provided treasury shard accounts.
/// 3. For each shard, checks that it is initialized, writable, and owned by the program.
/// 4. Validates that each shard account matches a valid treasury shard PDA.
/// 5. Transfers any lamports above the rent-exempt minimum from the shard account to the treasury wallet.
///
/// Effect:
/// - Consolidates all available treasury funds from the shards into the treasury account.
/// - Leaves each shard account at the minimum rent-exempt balance.
///
/// Notes:
/// - The caller supplies the shard index for each account, so exactly one PDA is
///   validated per shard instead of scanning the whole shard space.
///
/// # Parameters
/// - `program_id` — this program's address, used for shard PDA/ownership.
/// - `accounts` — `[settings, treasury_wallet, shard_accounts...]`.
/// - `shard_indices` — index per shard account, positionally paired with the
///   trailing shard accounts.
///
/// # Returns
/// - `Ok(())` once each shard's excess is moved to the treasury wallet.
/// - `ProtocolError::InvalidTreasury`/`NothingToSweep`/`InvalidShard`, or
///   PDA/ownership errors.
pub fn process_sweep_treasury(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    shard_indices: Vec<u16>,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let settings_account = next_account_info(account_info_iter)?;
    let treasury_wallet = next_account_info(account_info_iter)?;

    let settings = load_settings(program_id, settings_account)?;
    if *treasury_wallet.key != settings.treasury {
        return Err(ProtocolError::InvalidTreasury.into());
    }
    assert_writable(treasury_wallet)?;

    let rent = Rent::get()?;
    let shard_rent_min = rent.minimum_balance(0);

    let remaining = account_info_iter.as_slice();
    if remaining.is_empty() {
        return Err(ProtocolError::NothingToSweep.into());
    }
    if remaining.len() != shard_indices.len() {
        return Err(ProtocolError::InvalidShard.into());
    }

    for (shard_account, shard_idx) in remaining.iter().zip(shard_indices.iter()) {
        if is_uninitialized(shard_account) {
            continue;
        }
        assert_writable(shard_account)?;
        assert_owned_by(shard_account, program_id)?;

        validate_treasury_shard(program_id, shard_account, *shard_idx)?;

        sweep_shard_excess(shard_account, treasury_wallet, shard_rent_min)?;
    }

    Ok(())
}
