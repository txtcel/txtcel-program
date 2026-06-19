use borsh::BorshSerialize;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    program::invoke,
    pubkey::Pubkey,
    sysvar::{rent::Rent, Sysvar},
};
use solana_system_interface::instruction as system_instruction;

use crate::error::ProtocolError;
use crate::state::*;

pub fn process_create_root_alloc(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    message_fee: u64,
    treasury_shard_idx: u16,
    title: Vec<u8>,
) -> ProgramResult {
    if title.len() > MAX_TITLE_LEN {
        return Err(ProtocolError::TextTooLong.into());
    }

    let account_info_iter = &mut accounts.iter();
    let payer = next_account_info(account_info_iter)?;
    let thread_account = next_account_info(account_info_iter)?;
    let alloc_account = next_account_info(account_info_iter)?;
    let settings_account = next_account_info(account_info_iter)?;
    let treasury_shard = next_account_info(account_info_iter)?;
    let system_program_account = next_account_info(account_info_iter)?;

    assert_signer(payer)?;
    // The thread account is a fresh keypair that owns its address; it must sign
    // its own creation. There is no global counter, so channel creation has no
    // shared writable account and is fully parallelizable.
    assert_signer(thread_account)?;
    assert_writable(thread_account)?;
    assert_writable(alloc_account)?;
    assert_writable(treasury_shard)?;
    assert_system_program(system_program_account)?;

    let thread_key = *thread_account.key;

    let settings = load_settings(program_id, settings_account)?;

    let (expected_alloc, alloc_bump) = derive_alloc_pda(program_id, &thread_key, 0);

    if *alloc_account.key != expected_alloc {
        return Err(ProtocolError::InvalidPda.into());
    }

    let treasury_bump = validate_treasury_shard(program_id, treasury_shard, treasury_shard_idx)?;

    assert_uninitialized(thread_account)?;
    assert_uninitialized(alloc_account)?;

    ensure_shard_initialized(
        program_id,
        payer,
        treasury_shard,
        system_program_account,
        &[TREASURY_SHARD_SEED, &treasury_shard_idx.to_le_bytes()],
        treasury_bump,
    )?;

    let rent = Rent::get()?;

    let thread_size = ThreadNode::size(title.len());
    let thread_lamports = rent.minimum_balance(thread_size);

    invoke(
        &system_instruction::create_account(
            payer.key,
            thread_account.key,
            thread_lamports,
            thread_size as u64,
            program_id,
        ),
        &[payer.clone(), thread_account.clone(), system_program_account.clone()],
    )?;

    let alloc_size = AllocNode::size();
    let alloc_lamports = rent.minimum_balance(alloc_size);
    let combined_rent = thread_lamports
        .checked_add(alloc_lamports)
        .ok_or(ProtocolError::InvalidAccountData)?;

    collect_base_fee(
        combined_rent,
        settings.base_fee_bps,
        payer,
        treasury_shard,
        system_program_account,
    )?;

    create_pda_account(
        program_id,
        payer,
        alloc_account,
        system_program_account,
        alloc_size,
        &[ALLOC_SEED, thread_key.as_ref(), &0u32.to_le_bytes(), &[alloc_bump]],
    )?;

    let thread = ThreadNode {
        tag: TAG_THREAD,
        alloc_count: 1,
        last_alloc_seq: 0,
        author: *payer.key,
        message_fee,
        like_fee: 0,
        title,
    };

    thread.serialize(&mut &mut thread_account.data.borrow_mut()[..])?;

    let alloc = AllocNode {
        tag: TAG_ALLOC,
        thread: thread_key,
        alloc_seq: 0,
        upper_alloc_seq: INDEX_NONE,
        next_alloc_seq: INDEX_NONE,
    };

    alloc.serialize(&mut &mut alloc_account.data.borrow_mut()[..])?;

    Ok(())
}
