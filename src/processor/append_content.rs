use borsh::BorshSerialize;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    program::invoke,
    pubkey::Pubkey,
    sysvar::{clock::Clock, rent::Rent, Sysvar},
};
use solana_system_interface::instruction as system_instruction;

use crate::error::ProtocolError;
use crate::state::*;

pub fn process_append_content(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    chunk: Vec<u8>,
    treasury_shard_idx: u16,
    author_fee_shard_idx: u8,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let payer = next_account_info(account_info_iter)?;
    let content_account = next_account_info(account_info_iter)?;
    let thread_account = next_account_info(account_info_iter)?;
    let settings_account = next_account_info(account_info_iter)?;
    let treasury_shard = next_account_info(account_info_iter)?;
    let author_fee_shard = next_account_info(account_info_iter)?;
    let system_program_account = next_account_info(account_info_iter)?;

    assert_signer(payer)?;
    assert_writable(content_account)?;
    assert_owned_by(content_account, program_id)?;
    assert_writable(treasury_shard)?;
    assert_writable(author_fee_shard)?;
    assert_system_program(system_program_account)?;

    // Validates the thread account (owner + tag); its key is the channel id.
    let _thread = load_thread(program_id, thread_account)?;
    let settings = load_settings(program_id, settings_account)?;

    let thread_key = *thread_account.key;

    let treasury_shard_bump = validate_treasury_shard(program_id, treasury_shard, treasury_shard_idx)?;
    // The author-fee shard is still validated for a stable account layout, but
    // appends no longer pay the author's per-message fee (charged once on post).
    validate_author_fee_shard(program_id, &thread_key, author_fee_shard, author_fee_shard_idx)?;

    // `load_content` verifies owner + TAG_CONTENT + the content PDA derivation,
    // so a forged or substituted content account is rejected.
    let mut content = load_content(program_id, content_account)?;

    // Bind the content to the supplied thread so the appender cannot pass a
    // mismatched thread to dodge the base fee.
    if content.header.thread != thread_key {
        return Err(ProtocolError::ThreadMismatch.into());
    }

    if content.header.author != *payer.key {
        return Err(ProtocolError::Unauthorized.into());
    }

    const APPEND_WINDOW_SECS: i64 = 120;
    let now = Clock::get()?.unix_timestamp;
    if now - content.header.created_at > APPEND_WINDOW_SECS {
        return Err(ProtocolError::AppendWindowExpired.into());
    }

    // Appending raw bytes to the opaque body. This is meaningful for byte-stream
    // kinds (e.g. text); structured kinds simply choose not to use append.
    let new_body_len = content.body.len() + chunk.len();
    if new_body_len > MAX_BODY_LEN {
        return Err(ProtocolError::TextTooLong.into());
    }

    let old_size = ContentNode::size(content.body.len());
    content.body.extend_from_slice(&chunk);
    let new_size = ContentNode::size(content.body.len());

    content_account.resize(new_size)?;

    let rent = Rent::get()?;
    let required = rent.minimum_balance(new_size);
    let current = content_account.lamports();

    if required > current {
        let diff = required - current;
        invoke(
            &system_instruction::transfer(payer.key, content_account.key, diff),
            &[
                payer.clone(),
                content_account.clone(),
                system_program_account.clone(),
            ],
        )?;
    }

    let rent_delta = rent.minimum_balance(new_size).saturating_sub(rent.minimum_balance(old_size));

    if rent_delta > 0 {
        // Ensure the destination shards are program-owned before transferring
        // fees. Otherwise lamports could be sent to an uninitialized,
        // system-owned PDA, locking the funds and permanently bricking the
        // shard (it could never be `create_account`-ed afterwards).
        ensure_shard_initialized(
            program_id,
            payer,
            treasury_shard,
            system_program_account,
            &[TREASURY_SHARD_SEED, &treasury_shard_idx.to_le_bytes()],
            treasury_shard_bump,
        )?;

        // Only the platform base fee applies to appends. The author's
        // per-message fee is a fixed amount charged once when the slot is
        // filled, so growing the same message does not re-charge it.
        collect_base_fee(
            rent_delta,
            settings.base_fee_bps,
            payer,
            treasury_shard,
            system_program_account,
        )?;
    }

    content.serialize(&mut &mut content_account.data.borrow_mut()[..])?;

    Ok(())
}
