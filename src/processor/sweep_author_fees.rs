use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
    sysvar::{rent::Rent, Sysvar},
};

use crate::error::ProtocolError;
use crate::state::*;

/// Sweeps accumulated author fees from all fee shard accounts into the author's wallet.
///
/// Steps:
/// 1. Verifies that the provided author wallet matches the thread's author and signed the transaction.
/// 2. Iterates over the provided shard accounts.
/// 3. For each shard, checks that it is initialized, writable, and owned by the program.
/// 4. Confirms that each shard account matches a valid author fee PDA for the thread.
/// 5. Transfers any lamports above the rent-exempt minimum from the shard account to the author wallet.
///
/// Effect:
/// - Consolidates all available author fees from the shards into the author's account.
/// - Leaves each shard account at the minimum rent-exempt balance.
///
/// # Parameters
/// - `program_id` — this program's address, used for shard PDA/ownership.
/// - `accounts` — `[thread, author_wallet(signer), shard_accounts...]`.
/// - `shard_indices` — index per shard account, positionally paired with the
///   trailing shard accounts.
///
/// # Returns
/// - `Ok(())` once each shard's excess is moved to the author wallet.
/// - `ProtocolError::InvalidAuthor`/`NothingToSweep`/`InvalidShard`, or
///   PDA/ownership errors.
pub fn process_sweep_author_fees(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    shard_indices: Vec<u8>,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let thread_account = next_account_info(account_info_iter)?;
    let author_wallet = next_account_info(account_info_iter)?;

    let thread = load_thread(program_id, thread_account)?;
    if *author_wallet.key != thread.author {
        return Err(ProtocolError::InvalidAuthor.into());
    }

    assert_signer(author_wallet)?;
    assert_writable(author_wallet)?;

    let rent = Rent::get()?;
    let shard_rent_min = rent.minimum_balance(0);
    let thread_key = *thread_account.key;

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

        validate_author_fee_shard(program_id, &thread_key, shard_account, *shard_idx)?;

        sweep_shard_excess(shard_account, author_wallet, shard_rent_min)?;
    }

    Ok(())
}
