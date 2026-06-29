use borsh::BorshSerialize;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
    sysvar::{clock::Clock, rent::Rent, Sysvar},
};

use crate::error::ProtocolError;
use crate::state::*;

/// Fills a content slot in a thread with new text.
///
/// This instruction owns a single responsibility — writing a content element and
/// collecting its fees. Growing the page chain (linking the next `AllocNode`) is
/// a separate concern handled exclusively by `prepare_alloc`.
///
/// Steps:
/// 1. Validates text length, candidate slots, and required accounts (payer, thread, settings, treasury/author fee shards, system program).
/// 2. Loads the thread and program settings, validates treasury and author fee shards via PDAs.
/// 3. Checks optional access control: verifies whitelist/blacklist restrictions on the payer.
/// 4. Iterates over candidate slots to find the first uninitialized slot matching the allocation sequence and slot index.
///    - Creates a ContentNode account via PDA (invoke_signed) with rent-exempt lamports.
///    - Serializes the content into the account.
///    - Collects base protocol fee and author fee (if applicable) into respective shards.
/// 5. Returns an error if no candidate slot was filled or any PDA/account validations fail.
///
/// Notes:
/// - The body is opaque to the program: only its length is bounded so any message
///   `kind` (including ones added after deployment) can be stored. A known kind
///   additionally runs its own typed validation.
/// - The remaining-accounts tail is `[candidates...] [access] [entry]`. The
///   `access` and `entry` PDAs are MANDATORY and at fixed positions; both are
///   derived by the program (the entry PDA is bound to `payer`), so access control
///   cannot be bypassed by omitting or substituting accounts.
/// - Gating is enforced only when the thread opted in (`enabled`) AND there is
///   something to gate against: a non-empty whitelist or a paid entry fee. An
///   enabled thread with an empty whitelist and no entry fee stays open to
///   everyone; blacklisted (`ACCESS_DENIED`) wallets are always rejected. A
///   fee-exempt entry supersedes plain allow: it grants access and waives the fee.
/// - The thread author always posts for free; other wallets pay the fixed
///   per-message author fee (lamports, set by the author) unless they hold a
///   fee-exempt membership entry. Fees are computed up front and capped by
///   `max_fee` (slippage protection against the author/admin front-running a fee
///   change between submission and execution); the platform base fee is a
///   percentage of the slot's rent.
///
/// Invariants after success:
/// - One content slot is filled with text.
/// - Fees have been collected into treasury and author fee shards.
/// - The allocation chain is never touched (use `prepare_alloc` to link pages).
///
/// # Parameters
/// - `program_id` — this program's address, used for PDA derivation/ownership.
/// - `accounts` — `[payer, thread, settings, treasury_shard, author_fee_shard,
///   system, candidates..., access, entry]`.
/// - `kind` — message-type discriminator; known kinds get extra validation.
/// - `body` — opaque payload bytes, capped at `MAX_BODY_LEN`.
/// - `candidates` — slot targets tried in order until a free one is filled.
/// - `treasury_shard_idx` — treasury shard collecting the base fee.
/// - `author_fee_shard_idx` — shard collecting the per-message author fee.
/// - `reply_alloc_seq` — alloc seq of the replied-to message (threading).
/// - `reply_slot` — slot of the replied-to message (threading).
/// - `max_fee` — slippage cap on the total fee.
///
/// # Returns
/// - `Ok(())` once one slot is filled and fees collected.
/// - `ProtocolError::TextTooLong`/`InvalidCandidateCount`/`AccessDenied`/
///   `NoFreeSlot`/`FeeExceedsMax`, or PDA/validation errors.
#[allow(clippy::too_many_arguments)]
pub fn process_fill_slot(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    kind: u16,
    body: Vec<u8>,
    candidates: Vec<CandidateSlot>,
    treasury_shard_idx: u16,
    author_fee_shard_idx: u8,
    reply_alloc_seq: u32,
    reply_slot: u8,
    max_fee: u64,
) -> ProgramResult {
    if body.len() > MAX_BODY_LEN {
        return Err(ProtocolError::TextTooLong.into());
    }
    if ContentKind::from_u16(kind) == ContentKind::Text {
        TextBody::new(body.clone()).validate()?;
    }

    if candidates.is_empty() {
        return Err(ProtocolError::InvalidCandidateCount.into());
    }

    let account_info_iter = &mut accounts.iter();
    let payer = next_account_info(account_info_iter)?;
    let thread_account = next_account_info(account_info_iter)?;
    let settings_account = next_account_info(account_info_iter)?;
    let treasury_shard = next_account_info(account_info_iter)?;
    let author_fee_shard = next_account_info(account_info_iter)?;
    let system_program_account = next_account_info(account_info_iter)?;

    assert_signer(payer)?;
    assert_fee_shard_accounts(treasury_shard, author_fee_shard, system_program_account)?;

    let thread = load_thread(program_id, thread_account)?;
    let thread_key = *thread_account.key;
    let settings = load_settings(program_id, settings_account)?;

    let (treasury_shard_bump, author_fee_shard_bump) = validate_fee_shards(
        program_id,
        treasury_shard,
        author_fee_shard,
        &thread_key,
        treasury_shard_idx,
        author_fee_shard_idx,
    )?;

    let remaining = account_info_iter.as_slice();
    let n_candidates = candidates.len();

    if remaining.len() < n_candidates + 2 {
        return Err(ProtocolError::InvalidCandidateCount.into());
    }

    let candidate_accounts = &remaining[..n_candidates];
    let access_account = &remaining[n_candidates];
    let entry_account = &remaining[n_candidates + 1];

    let (expected_access, _) = derive_access_pda(program_id, &thread_key);
    assert_pda(access_account, &expected_access)?;
    let (expected_entry, _) = derive_access_entry_pda(program_id, &thread_key, payer.key);
    assert_pda(entry_account, &expected_entry)?;

    let mut gating_enabled = false;
    if !is_uninitialized(access_account) {
        assert_owned_by(access_account, program_id)?;
        let access = load_thread_access(access_account)?;
        if access.thread != thread_key {
            return Err(ProtocolError::ThreadMismatch.into());
        }
        gating_enabled = access.enabled && (access.whitelist_count > 0 || access.entry_fee > 0);
    }

    let mut allowed = *payer.key == thread.author;
    let mut fee_exempt = *payer.key == thread.author;
    if !is_uninitialized(entry_account) {
        let entry = load_bound_access_entry(program_id, entry_account, &thread_key, payer.key)?;
        if entry.status == ACCESS_DENIED {
            return Err(ProtocolError::AccessDenied.into());
        }
        if entry.status == ACCESS_ALLOWED || entry.status == ACCESS_FEE_EXEMPT {
            allowed = true;
        }
        if entry.status == ACCESS_FEE_EXEMPT {
            fee_exempt = true;
        }
    }

    if gating_enabled && !allowed {
        return Err(ProtocolError::AccessDenied.into());
    }

    ensure_fee_shards_initialized(
        program_id,
        payer,
        treasury_shard,
        author_fee_shard,
        system_program_account,
        &thread_key,
        treasury_shard_idx,
        author_fee_shard_idx,
        treasury_shard_bump,
        author_fee_shard_bump,
    )?;

    let mut filled = false;
    let rent = Rent::get()?;
    let created_at = Clock::get()?.unix_timestamp;

    for (i, candidate) in candidates.iter().enumerate() {
        if candidate.alloc_seq > thread.last_alloc_seq {
            return Err(ProtocolError::InvalidAllocSeq.into());
        }
        if candidate.slot as usize >= CONTENT_SLOTS {
            return Err(ProtocolError::InvalidSlot.into());
        }

        let content_account = &candidate_accounts[i];
        let (expected_content, content_bump) =
            derive_content_pda(program_id, &thread_key, candidate.alloc_seq, candidate.slot);

        assert_pda(content_account, &expected_content)?;

        if !is_uninitialized(content_account) {
            continue;
        }

        assert_writable(content_account)?;

        let content_size = ContentNode::size(body.len());
        let content_lamports = rent.minimum_balance(content_size);

        create_pda_account(
            program_id,
            payer,
            content_account,
            system_program_account,
            content_size,
            &[
                CONTENT_SEED,
                thread_key.as_ref(),
                &candidate.alloc_seq.to_le_bytes(),
                &[candidate.slot],
                &[content_bump],
            ],
        )?;

        let content = ContentNode {
            header: ContentHeader {
                tag: TAG_CONTENT,
                alloc_seq: candidate.alloc_seq,
                slot: candidate.slot,
                thread: thread_key,
                author: *payer.key,
                created_at,
                reply_alloc_seq,
                reply_slot,
            },
            kind,
            body,
        };

        content.serialize(&mut &mut content_account.data.borrow_mut()[..])?;

        let base_fee = ((content_lamports as u128)
            .checked_mul(settings.base_fee_bps as u128)
            .ok_or(ProtocolError::InvalidAccountData)? / 10_000) as u64;

        let total_author_fee = if fee_exempt {
            0
        } else {
            thread.message_fee
        };

        let total_fee = base_fee
            .checked_add(total_author_fee)
            .ok_or(ProtocolError::InvalidAccountData)?;
        if total_fee > max_fee {
            return Err(ProtocolError::FeeExceedsMax.into());
        }

        collect_base_fee(
            content_lamports,
            settings.base_fee_bps,
            payer,
            treasury_shard,
            system_program_account,
        )?;

        if total_author_fee > 0 {
            transfer_fee_split(
                total_author_fee,
                settings.author_fee_cut_bps,
                payer,
                author_fee_shard,
                treasury_shard,
                system_program_account,
            )?;
        }

        filled = true;

        break;
    }

    if !filled {
        return Err(ProtocolError::NoFreeSlot.into());
    }

    Ok(())
}
