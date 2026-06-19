use borsh::BorshSerialize;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
};

use crate::error::ProtocolError;
use crate::state::*;

/// Updates the fixed per-message fee for a specific thread.
///
/// Steps:
/// 1. Confirms that the authority signing the transaction is the thread's author.
/// 2. Loads the thread account, updates the `message_fee` field (in lamports)
///    with the new value, and serializes the updated data back to the account.
///
/// After execution:
/// - Non-authors pay the new fixed amount for every message posted in the thread.
pub fn process_set_message_fee(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    fee: u64,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let authority = next_account_info(account_info_iter)?;
    let thread_account = next_account_info(account_info_iter)?;

    assert_signer(authority)?;
    assert_writable(thread_account)?;

    let mut thread = load_thread(program_id, thread_account)?;

    if *authority.key != thread.author {
        return Err(ProtocolError::Unauthorized.into());
    }

    thread.message_fee = fee;
    thread.serialize(&mut &mut thread_account.data.borrow_mut()[..])?;

    Ok(())
}
