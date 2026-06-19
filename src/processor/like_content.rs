use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
};
use solana_system_interface::program as system_program;

use crate::error::ProtocolError;
use crate::state::*;

/// Processes a "like" action for a specific content slot in a thread.
///
/// Steps:
/// 1. Validates input parameters: slot index within allowed range, PDA accounts for content, likes, thread, treasury, and author fee shards.
/// 2. Ensures the payer is a signer and writable where needed, and system program is correct.
/// 3. Loads the target content and verifies it matches the expected PDA derived from the thread seed, allocation sequence, and slot.
/// 4. Loads the thread and program settings, checking the thread seed matches the content.
/// 5. Validates and initializes the treasury and author fee shards to collect fees if needed.
/// 6. Derives and, if necessary, creates the likes PDA account to store per-slot like counts. Initializes it with zero counts.
/// 7. Loads the likes data, increments the count for the given slot, and serializes it back to the likes account.
/// 8. If the thread has a nonzero like fee and the liker is not the content author, the fee is split and transferred between the author and treasury shards.
///
/// After execution:
/// - The likes count for the specific slot is updated.
/// - Appropriate fees are collected and distributed according to thread settings.
/// - The likes account is initialized if it did not exist before.
#[allow(clippy::too_many_arguments)]
pub fn process_like_content(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    alloc_seq: u32,
    slot: u8,
    treasury_shard_idx: u16,
    author_fee_shard_idx: u8,
    max_fee: u64,
) -> ProgramResult {
    if slot as usize >= CONTENT_SLOTS {
        return Err(ProtocolError::InvalidSlot.into());
    }

    let account_info_iter = &mut accounts.iter();
    let payer = next_account_info(account_info_iter)?;
    let likes_account = next_account_info(account_info_iter)?;
    let content_account = next_account_info(account_info_iter)?;
    let thread_account = next_account_info(account_info_iter)?;
    let settings_account = next_account_info(account_info_iter)?;
    let treasury_shard = next_account_info(account_info_iter)?;
    let author_fee_shard = next_account_info(account_info_iter)?;
    let system_program_account = next_account_info(account_info_iter)?;

    assert_signer(payer)?;
    assert_writable(likes_account)?;
    assert_writable(treasury_shard)?;
    assert_writable(author_fee_shard)?;
    assert_system_program(system_program_account)?;

    let thread_key = *thread_account.key;

    let content = load_content(program_id, content_account)?;

    let (expected_content, _) = derive_content_pda(program_id, &thread_key, alloc_seq, slot);

    if *content_account.key != expected_content {
        return Err(ProtocolError::InvalidPda.into());
    }

    let thread = load_thread(program_id, thread_account)?;

    let settings = load_settings(program_id, settings_account)?;

    let treasury_shard_bump = validate_treasury_shard(program_id, treasury_shard, treasury_shard_idx)?;
    let author_fee_shard_bump = validate_author_fee_shard(program_id, &thread_key, author_fee_shard, author_fee_shard_idx)?;

    ensure_shard_initialized(
        program_id,
        payer,
        treasury_shard,
        system_program_account,
        &[TREASURY_SHARD_SEED, &treasury_shard_idx.to_le_bytes()],
        treasury_shard_bump,
    )?;
    ensure_shard_initialized(
        program_id,
        payer,
        author_fee_shard,
        system_program_account,
        &[AUTHOR_FEE_SEED, thread_key.as_ref(), &[author_fee_shard_idx]],
        author_fee_shard_bump,
    )?;

    let (expected_likes, likes_bump) = derive_likes_pda(program_id, &thread_key, alloc_seq);

    if *likes_account.key != expected_likes {
        return Err(ProtocolError::InvalidPda.into());
    }

    if system_program::check_id(likes_account.owner) && likes_account.data_len() == 0 {
        let size = AllocLikes::size();
        create_pda_account(
            program_id,
            payer,
            likes_account,
            system_program_account,
            size,
            &[LIKES_SEED, thread_key.as_ref(), &alloc_seq.to_le_bytes(), &[likes_bump]],
        )?;
        let likes = AllocLikes {
            tag: TAG_LIKES,
            alloc_seq,
            counts: [0u32; NEXT_ALLOC_INDEX],
        };
        likes.serialize(&mut &mut likes_account.data.borrow_mut()[..])?;
    }

    assert_owned_by(likes_account, program_id)?;

    let mut likes = AllocLikes::try_from_slice(&likes_account.data.borrow()).map_err(|_| ProtocolError::InvalidAccountData)?;

    if likes.tag != TAG_LIKES {
        return Err(ProtocolError::InvalidTag.into());
    }

    likes.counts[slot as usize] = likes.counts[slot as usize].saturating_add(1);
    likes.serialize(&mut &mut likes_account.data.borrow_mut()[..])?;

    if thread.like_fee > 0 && *payer.key != content.header.author {
        if thread.like_fee > max_fee {
            return Err(ProtocolError::FeeExceedsMax.into());
        }
        transfer_fee_split(
            thread.like_fee,
            settings.like_cut_bps,
            payer,
            author_fee_shard,
            treasury_shard,
            system_program_account,
        )?;
    }

    Ok(())
}
