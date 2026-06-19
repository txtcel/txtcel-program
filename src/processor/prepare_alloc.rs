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
/// 3. Checks that the current allocation is not already linked to a next allocation.
/// 4. Computes the next allocation sequence number and derives the PDA for the new allocation account, validating it matches the expected address.
/// 5. Ensures the new allocation account is uninitialized, then creates it with the required rent-exempt lamports via `invoke_signed`.
/// 6. Initializes the new allocation node with appropriate links to the previous allocation and serializes it to the new account.
/// 7. Updates the current allocation's `next_alloc_seq` to point to the new allocation and serializes it back.
/// 8. Updates the thread's allocation count and last allocation sequence, then serializes the thread account.
///
/// After execution:
/// - A new allocation node is created and linked to the previous node.
/// - The thread's metadata reflects the newly created allocation.
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

    let mut current_alloc = load_alloc(program_id, current_alloc_account)?;

    if current_alloc.thread != thread_key {
        return Err(ProtocolError::ThreadMismatch.into());
    }

    if current_alloc.alloc_seq != alloc_seq {
        return Err(ProtocolError::InvalidAllocSeq.into());
    }

    if current_alloc.next_alloc_seq != INDEX_NONE {
        return Err(ProtocolError::AllocAlreadyLinked.into());
    }

    let mut thread = load_thread(program_id, thread_account)?;

    let new_seq = alloc_seq
        .checked_add(1)
        .ok_or(ProtocolError::InvalidAccountData)?;

    let (expected_new, new_bump) = derive_alloc_pda(program_id, &thread_key, new_seq);

    if *new_alloc_account.key != expected_new {
        return Err(ProtocolError::InvalidPda.into());
    }

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
        upper_alloc_seq: alloc_seq,
        next_alloc_seq: INDEX_NONE,
    };

    new_alloc.serialize(&mut &mut new_alloc_account.data.borrow_mut()[..])?;

    current_alloc.next_alloc_seq = new_seq;
    current_alloc.serialize(&mut &mut current_alloc_account.data.borrow_mut()[..])?;

    thread.alloc_count = thread.alloc_count.checked_add(1).ok_or(ProtocolError::InvalidAccountData)?;

    thread.last_alloc_seq = new_seq;
    thread.serialize(&mut &mut thread_account.data.borrow_mut()[..])?;

    Ok(())
}
