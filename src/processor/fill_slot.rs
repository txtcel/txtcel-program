use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
    sysvar::{clock::Clock, rent::Rent, Sysvar},
};

use crate::error::ProtocolError;
use crate::state::*;

/// Fills a content slot in a thread with new text and optionally extends the allocation chain.
///
/// Steps:
/// 1. Validates text length, candidate slots, and required accounts (payer, thread, settings, treasury/author fee shards, system program).
/// 2. Loads the thread and program settings, validates treasury and author fee shards via PDAs.
/// 3. Checks optional access control: verifies whitelist/blacklist restrictions on the payer.
/// 4. Iterates over candidate slots to find the first uninitialized slot matching the allocation sequence and slot index.
///    - Creates a ContentNode account via PDA (invoke_signed) with rent-exempt lamports.
///    - Serializes the content into the account.
///    - Collects base protocol fee and author fee (if applicable) into respective shards.
/// 5. If `extend` is true, attempts to auto-create a new allocation node:
///    - Validates current allocation, derives the next allocation PDA, and ensures it is uninitialized.
///    - Creates new AllocNode account and updates links in current allocation and thread metadata.
/// 6. Returns an error if no candidate slot was filled or any PDA/account validations fail.
///
/// Invariants after success:
/// - One content slot is filled with text.
/// - Fees have been collected into treasury and author fee shards.
/// - Thread and allocation chain updated if auto-extend was applied.
#[allow(clippy::too_many_arguments)]
pub fn process_fill_slot(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    kind: u16,
    body: Vec<u8>,
    candidates: Vec<CandidateSlot>,
    extend: bool,
    treasury_shard_idx: u16,
    author_fee_shard_idx: u8,
    reply_alloc_seq: u32,
    reply_slot: u8,
    max_fee: u64,
) -> ProgramResult {
    // The body is opaque to the program: only its length is bounded here so any
    // message `kind` (including ones added after deployment) can be stored. A
    // known kind additionally runs its own typed validation below.
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
    assert_writable(treasury_shard)?;
    assert_writable(author_fee_shard)?;
    assert_system_program(system_program_account)?;

    let thread = load_thread(program_id, thread_account)?;
    let thread_key = *thread_account.key;
    let settings = load_settings(program_id, settings_account)?;

    let treasury_shard_bump = validate_treasury_shard(program_id, treasury_shard, treasury_shard_idx)?;
    let author_fee_shard_bump = validate_author_fee_shard(program_id, &thread_key, author_fee_shard, author_fee_shard_idx)?;

    let remaining = account_info_iter.as_slice();
    let n_candidates = candidates.len();

    // remaining layout: [candidates...] [access] [entry] [optional: current_alloc, new_alloc]
    // access and entry are MANDATORY and at fixed positions so access control
    // cannot be bypassed by omitting accounts.
    if remaining.len() < n_candidates + 2 {
        return Err(ProtocolError::InvalidCandidateCount.into());
    }

    let candidate_accounts = &remaining[..n_candidates];
    let access_account = &remaining[n_candidates];
    let entry_account = &remaining[n_candidates + 1];
    let extend_slice = &remaining[n_candidates + 2..];

    // Both PDAs are derived by the program, so the caller can neither omit nor
    // substitute them. The entry PDA is bound to `payer`.
    let (expected_access, _) = derive_access_pda(program_id, &thread_key);
    if *access_account.key != expected_access {
        return Err(ProtocolError::InvalidPda.into());
    }
    let (expected_entry, _) = derive_access_entry_pda(program_id, &thread_key, payer.key);
    if *entry_account.key != expected_entry {
        return Err(ProtocolError::InvalidPda.into());
    }

    // Gating is only enforced when the thread opted in (`enabled`) AND there is
    // something to gate against: a non-empty whitelist or a paid entry fee. An
    // enabled thread with an empty whitelist and no entry fee stays open to
    // everyone (blacklisted wallets are still rejected above via the entry).
    let mut gating_enabled = false;
    if !is_uninitialized(access_account) {
        assert_owned_by(access_account, program_id)?;
        let access = load_thread_access(access_account)?;
        if access.thread != thread_key {
            return Err(ProtocolError::ThreadMismatch.into());
        }
        gating_enabled = access.enabled && (access.whitelist_count > 0 || access.entry_fee > 0);
    }

    // The thread author always posts for free; other wallets pay the per-message
    // author fee unless they hold a fee-exempt membership entry.
    let mut allowed = *payer.key == thread.author;
    let mut fee_exempt = *payer.key == thread.author;
    if !is_uninitialized(entry_account) {
        assert_owned_by(entry_account, program_id)?;
        let entry = load_access_entry(entry_account)?;
        if entry.thread != thread_key || entry.wallet != *payer.key {
            return Err(ProtocolError::ThreadMismatch.into());
        }
        if entry.status == ACCESS_DENIED {
            return Err(ProtocolError::AccessDenied.into());
        }
        // Fee-exempt supersedes plain allow: it grants access and waives the fee.
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

    // Try each candidate
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

        if *content_account.key != expected_content {
            return Err(ProtocolError::InvalidPda.into());
        }

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

        // Compute fees up front and enforce the caller's slippage cap. This
        // protects against the author/admin front-running fee changes between
        // submission and execution.
        //
        // The author fee is a fixed per-message amount (in lamports) set by the
        // thread author, charged only when the poster is not the author. The
        // platform base fee remains a percentage of the slot's rent.
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

    // Auto-extend
    if extend {
        if extend_slice.len() >= 2 {
            let current_alloc_account = &extend_slice[0];
            let new_alloc_account = &extend_slice[1];

            assert_writable(current_alloc_account)?;
            assert_writable(thread_account)?;

            if is_uninitialized(new_alloc_account) {
                assert_writable(new_alloc_account)?;

                let mut current_alloc = load_alloc(program_id, current_alloc_account)?;

                if current_alloc.thread != thread_key {
                    return Err(ProtocolError::ThreadMismatch.into());
                }

                if current_alloc.next_alloc_seq != INDEX_NONE {
                    return Err(ProtocolError::AllocAlreadyLinked.into());
                }

                let new_seq = current_alloc.alloc_seq
                    .checked_add(1)
                    .ok_or(ProtocolError::InvalidAccountData)?;

                let (expected_new_alloc, new_alloc_bump) = derive_alloc_pda(program_id, &thread_key, new_seq);

                if *new_alloc_account.key != expected_new_alloc {
                    return Err(ProtocolError::InvalidPda.into());
                }

                let alloc_size = AllocNode::size();

                create_pda_account(
                    program_id,
                    payer,
                    new_alloc_account,
                    system_program_account,
                    alloc_size,
                    &[ALLOC_SEED, thread_key.as_ref(), &new_seq.to_le_bytes(), &[new_alloc_bump]],
                )?;

                let new_alloc = AllocNode {
                    tag: TAG_ALLOC,
                    thread: thread_key,
                    alloc_seq: new_seq,
                    upper_alloc_seq: current_alloc.alloc_seq,
                    next_alloc_seq: INDEX_NONE,
                };

                new_alloc.serialize(&mut &mut new_alloc_account.data.borrow_mut()[..])?;

                current_alloc.next_alloc_seq = new_seq;
                current_alloc.serialize(&mut &mut current_alloc_account.data.borrow_mut()[..])?;

                let mut thread_mut = ThreadNode::try_from_slice(&thread_account.data.borrow()).map_err(|_| ProtocolError::InvalidAccountData)?;

                thread_mut.alloc_count = thread_mut.alloc_count.checked_add(1).ok_or(ProtocolError::InvalidAccountData)?;
                thread_mut.last_alloc_seq = new_seq;
                thread_mut.serialize(&mut &mut thread_account.data.borrow_mut()[..])?;
            }
            // If new_alloc is already initialized, silently skip
        }
    }

    Ok(())
}
