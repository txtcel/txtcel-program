use borsh::BorshSerialize;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
};

use crate::error::ProtocolError;
use crate::state::*;

/// Prepares a new allocation node for a thread.
///
/// Steps:
/// 1. Validates that the payer is a signer and that the relevant accounts (current alloc, new alloc, thread) are writable.
/// 2. Loads the current allocation node and thread, verifying that the seed and allocation sequence match expected values.
/// 3. Confirms the current allocation is the chain's true tail (its `alloc_seq` equals the thread's `last_alloc_seq`).
/// 4. Computes the next allocation sequence number and derives the PDA for the new allocation account, validating it matches the expected address.
/// 5. Ensures the new allocation account is uninitialized, then creates it with the required rent-exempt lamports via `invoke_signed`.
/// 6. Initializes the new allocation node at `alloc_seq + 1` and serializes it to the new account.
/// 7. Updates the thread's allocation count and last allocation sequence, then serializes the thread account.
///
/// After execution:
/// - A new allocation node is appended at the tail of the chain.
/// - The thread's metadata reflects the newly created allocation.
///
/// Notes:
/// - The alloc chain stores no forward/back links: nodes are addressed purely
///   by their PDA `[ALLOC_SEED, thread, alloc_seq]` and numbered densely, so
///   `ThreadNode.last_alloc_seq` alone marks the tail.
/// - Only the true tail (matching the thread's `last_alloc_seq`) may be
///   extended. Without this check, a stale node could fork the chain and
///   regress `last_alloc_seq`.
///
/// # Parameters
/// - `program_id` — this program's address, used for PDA derivation/ownership.
/// - `accounts` — `[payer, current_alloc, new_alloc, thread, system]`.
/// - `alloc_seq` — sequence of the current tail alloc to extend; must equal the
///   thread's `last_alloc_seq`.
///
/// # Returns
/// - `Ok(())` once the new alloc is created and the thread updated.
/// - `ProtocolError::ThreadMismatch`/`InvalidAllocSeq`, or PDA/account-creation
///   errors.
pub fn process_prepare_alloc(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    alloc_seq: u32,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let payer = next_account_info(account_info_iter)?;
    let current_alloc_account = next_account_info(account_info_iter)?;
    let new_alloc_account = next_account_info(account_info_iter)?;
    let thread_account = next_account_info(account_info_iter)?;
    let system_program_account = next_account_info(account_info_iter)?;

    assert_signer(payer)?;
    assert_writable(current_alloc_account)?;
    assert_writable(new_alloc_account)?;
    assert_writable(thread_account)?;
    assert_system_program(system_program_account)?;

    let thread_key = *thread_account.key;

    let current_alloc = load_alloc(program_id, current_alloc_account)?;

    if current_alloc.thread != thread_key {
        return Err(ProtocolError::ThreadMismatch.into());
    }

    if current_alloc.alloc_seq != alloc_seq {
        return Err(ProtocolError::InvalidAllocSeq.into());
    }

    let mut thread = load_thread(program_id, thread_account)?;

    if current_alloc.alloc_seq != thread.last_alloc_seq {
        return Err(ProtocolError::InvalidAllocSeq.into());
    }

    let new_seq = alloc_seq
        .checked_add(1)
        .ok_or(ProtocolError::InvalidAccountData)?;

    let (expected_new, new_bump) = derive_alloc_pda(program_id, &thread_key, new_seq);

    assert_pda(new_alloc_account, &expected_new)?;

    assert_uninitialized(new_alloc_account)?;

    let alloc_size = AllocNode::size();

    create_pda_account(
        program_id,
        payer,
        new_alloc_account,
        system_program_account,
        alloc_size,
        &[ALLOC_SEED, thread_key.as_ref(), &new_seq.to_le_bytes(), &[new_bump]],
    )?;

    let new_alloc = AllocNode {
        tag: TAG_ALLOC,
        thread: thread_key,
        alloc_seq: new_seq,
    };

    new_alloc.serialize(&mut &mut new_alloc_account.data.borrow_mut()[..])?;

    thread.alloc_count = thread.alloc_count.checked_add(1).ok_or(ProtocolError::InvalidAccountData)?;

    thread.last_alloc_seq = new_seq;
    thread.serialize(&mut &mut thread_account.data.borrow_mut()[..])?;

    Ok(())
}
