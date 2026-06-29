use borsh::BorshSerialize;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
};

use crate::state::*;

/// Updates the "like" fee for a specific thread.
///
/// Steps:
/// 1. Loads the thread account and verifies the signer is the thread's author.
/// 2. Updates the `like_fee` field with the new fee amount.
/// 3. Serializes the updated thread account data back to the blockchain.
///
/// Effect:
/// - Any future likes on content within this thread will require the updated fee.
///
/// # Parameters
/// - `program_id` — this program's address, used for thread PDA/ownership.
/// - `accounts` — `[authority(thread author signer), thread]`.
/// - `fee` — new per-like fee in lamports.
///
/// # Returns
/// - `Ok(())` once the new like fee is persisted.
/// - Author/PDA validation errors from `load_author_thread`.
pub fn process_set_like_fee(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    fee: u64,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let authority = next_account_info(account_info_iter)?;
    let thread_account = next_account_info(account_info_iter)?;

    let mut thread = load_author_thread(program_id, authority, thread_account)?;

    thread.like_fee = fee;
    thread.serialize(&mut &mut thread_account.data.borrow_mut()[..])?;

    Ok(())
}
