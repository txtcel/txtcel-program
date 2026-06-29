use borsh::BorshSerialize;
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
///
/// # Parameters
/// - `program_id` — this program's address, used for PDA derivation/ownership.
/// - `accounts` — `[payer, likes, content, thread, settings, treasury_shard,
///   author_fee_shard, system]`.
/// - `alloc_seq` — alloc sequence of the liked content (locates the slot).
/// - `slot` — slot index within the alloc, bounded by `CONTENT_SLOTS`.
/// - `treasury_shard_idx` — treasury shard collecting the platform like cut.
/// - `author_fee_shard_idx` — shard collecting the author's like share.
/// - `max_fee` — slippage cap on the like fee.
///
/// # Returns
/// - `Ok(())` once the like is recorded and any fee is split.
/// - `ProtocolError::InvalidSlot`/`FeeExceedsMax`, or PDA/validation errors.
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
    assert_fee_shard_accounts(treasury_shard, author_fee_shard, system_program_account)?;

    let thread_key = *thread_account.key;

    let content = load_content(program_id, content_account)?;

    let (expected_content, _) = derive_content_pda(program_id, &thread_key, alloc_seq, slot);

    assert_pda(content_account, &expected_content)?;

    let thread = load_thread(program_id, thread_account)?;

    let settings = load_settings(program_id, settings_account)?;

    prepare_fee_shards(
        program_id,
        payer,
        treasury_shard,
        author_fee_shard,
        system_program_account,
        &thread_key,
        treasury_shard_idx,
        author_fee_shard_idx,
    )?;

    let (expected_likes, likes_bump) = derive_likes_pda(program_id, &thread_key, alloc_seq);

    assert_pda(likes_account, &expected_likes)?;

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
            counts: [0u32; CONTENT_SLOTS],
        };
        likes.serialize(&mut &mut likes_account.data.borrow_mut()[..])?;
    }

    assert_owned_by(likes_account, program_id)?;

    let mut likes = load_alloc_likes(likes_account)?;

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
