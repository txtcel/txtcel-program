//! Fee math and fee-shard plumbing shared by every fee-charging instruction.
//!
//! Fees are collected into sharded PDAs (treasury + per-thread author) to avoid
//! a single hot account. These helpers compute the author/platform split,
//! transfer lamports into the right shard, validate shard PDAs, lazily create
//! them, and sweep their excess — always in the fixed treasury-then-author
//! order so every call site behaves identically.

use solana_program::{
    account_info::AccountInfo,
    entrypoint::ProgramResult,
    program::invoke,
    program_error::ProgramError,
    pubkey::Pubkey,
    sysvar::{rent::Rent, Sysvar},
};
use solana_system_interface::instruction as system_instruction;

use crate::error::ProtocolError;

use super::account_utils::create_pda_account;
use super::asserts::{assert_pda, assert_system_program, assert_writable, is_uninitialized};
use super::constants::{AUTHOR_FEE_SEED, N_AUTHOR_FEE_SHARDS, N_TREASURY_SHARDS, TREASURY_SHARD_SEED};
use super::pda::{derive_author_fee_pda, derive_treasury_shard_pda};

/// Splits `amount` into `(author_receives, platform_cut)` by `cut_bps` basis
/// points, using saturating u128 math so an overflow can never mint lamports.
/// The platform cut rounds down, so the author absorbs any rounding remainder.
///
/// # Parameters
/// - `amount` — total lamports being split.
/// - `cut_bps` — platform's share in basis points (1/10_000).
/// # Returns
/// - `(author_receives, platform_cut)` lamports summing to `amount`.
pub fn compute_fee_split(amount: u64, cut_bps: u32) -> (u64, u64) {
    let platform = (amount as u128)
        .saturating_mul(cut_bps as u128)
        / 10_000;
    let platform = platform as u64;
    (amount.saturating_sub(platform), platform)
}

/// Transfers `amount` lamports from `payer` into a fee shard. If the shard is
/// below rent exemption it is topped up to at least the rent minimum in the same
/// transfer, so collecting a fee can never leave the shard rent-deficient. A
/// no-op when `amount` is zero.
///
/// # Parameters
/// - `amount` — fee lamports to collect.
/// - `payer` — funds the transfer.
/// - `shard` — destination fee shard account.
/// - `system_program_account` — System Program for the transfer CPI.
/// # Returns
/// - `Ok(())` once collected (or skipped for a zero fee), else the CPI error.
pub fn collect_fee_to_shard<'a>(
    amount: u64,
    payer: &AccountInfo<'a>,
    shard: &AccountInfo<'a>,
    system_program_account: &AccountInfo<'a>,
) -> ProgramResult {
    if amount == 0 {
        return Ok(());
    }

    let rent = Rent::get()?;
    let shard_rent_min = rent.minimum_balance(0);
    let fee = std::cmp::max(amount, shard_rent_min.saturating_sub(shard.lamports()));

    if fee > 0 {
        invoke(
            &system_instruction::transfer(payer.key, shard.key, fee),
            &[payer.clone(), shard.clone(), system_program_account.clone()],
        )?;
    }
    Ok(())
}

/// Charges the platform "base fee" — `base_fee_bps` of the rent paid to create
/// an account — into the treasury shard. Lets the platform take a cut
/// proportional to the rent a new account locks up.
///
/// # Parameters
/// - `rent_lamports` — rent the new account locks up; the fee base.
/// - `base_fee_bps` — platform base-fee rate in basis points.
/// - `payer` — funds the fee.
/// - `treasury_shard` — destination treasury shard.
/// - `system_program_account` — System Program for the transfer CPI.
/// # Returns
/// - `Ok(())` once the base fee is collected (or skipped when zero).
pub fn collect_base_fee<'a>(
    rent_lamports: u64,
    base_fee_bps: u32,
    payer: &AccountInfo<'a>,
    treasury_shard: &AccountInfo<'a>,
    system_program_account: &AccountInfo<'a>,
) -> ProgramResult {

    let base_fee = (rent_lamports as u128).saturating_mul(base_fee_bps as u128) / 10_000;

    collect_fee_to_shard(base_fee as u64, payer, treasury_shard, system_program_account)
}

/// Splits `amount` by `cut_bps` and routes the platform cut to the treasury
/// shard and the remainder to the author-fee shard, skipping any zero leg.
/// The single entry point for paying out a message/like fee.
///
/// # Parameters
/// - `amount` — total fee lamports to split and route.
/// - `cut_bps` — platform's share in basis points.
/// - `payer` — funds both legs of the transfer.
/// - `author_fee_shard` — destination for the author's share.
/// - `treasury_shard` — destination for the platform cut.
/// - `system_program_account` — System Program for the transfer CPIs.
/// # Returns
/// - `Ok(())` once both non-zero legs are collected, else the CPI error.
pub fn transfer_fee_split<'a>(
    amount: u64,
    cut_bps: u32,
    payer: &AccountInfo<'a>,
    author_fee_shard: &AccountInfo<'a>,
    treasury_shard: &AccountInfo<'a>,
    system_program_account: &AccountInfo<'a>,
) -> ProgramResult {
    let (author_receives, platform_cut) = compute_fee_split(amount, cut_bps);

    if platform_cut > 0 {
        collect_fee_to_shard(platform_cut, payer, treasury_shard, system_program_account)?;
    }

    if author_receives > 0 {
        collect_fee_to_shard(author_receives, payer, author_fee_shard, system_program_account)?;
    }

    Ok(())
}

// ── treasury shard validation ──

/// Bounds-checks the treasury shard index and asserts the passed account is the
/// canonical treasury-shard PDA, returning its bump. Guards against fees being
/// routed to a spoofed or out-of-range shard account.
///
/// # Parameters
/// - `program_id` — this program's id, for deriving the expected PDA.
/// - `shard_account` — the treasury shard account to validate.
/// - `shard_idx` — the shard index to check and derive.
/// # Returns
/// - `Ok(bump)` for the validated shard, or `InvalidShard`/`InvalidPda`.
pub fn validate_treasury_shard(
    program_id: &Pubkey,
    shard_account: &AccountInfo,
    shard_idx: u16,
) -> Result<u8, ProgramError> {
    if shard_idx >= N_TREASURY_SHARDS {
        return Err(ProtocolError::InvalidShard.into());
    }

    let (expected, bump) = derive_treasury_shard_pda(program_id, shard_idx);

    assert_pda(shard_account, &expected)?;

    Ok(bump)
}

/// Bounds-checks the author-fee shard index and asserts the passed account is
/// the canonical per-thread author-fee PDA, returning its bump. The author-side
/// counterpart of `validate_treasury_shard`.
///
/// # Parameters
/// - `program_id` — this program's id, for deriving the expected PDA.
/// - `thread` — the thread the author-fee shard belongs to.
/// - `shard_account` — the author-fee shard account to validate.
/// - `shard_idx` — the shard index to check and derive.
/// # Returns
/// - `Ok(bump)` for the validated shard, or `InvalidShard`/`InvalidPda`.
pub fn validate_author_fee_shard(
    program_id: &Pubkey,
    thread: &Pubkey,
    shard_account: &AccountInfo,
    shard_idx: u8,
) -> Result<u8, ProgramError> {
    if shard_idx >= N_AUTHOR_FEE_SHARDS {
        return Err(ProtocolError::InvalidShard.into());
    }

    let (expected, bump) = derive_author_fee_pda(program_id, thread, shard_idx);

    assert_pda(shard_account, &expected)?;

    Ok(bump)
}

/// Lazily creates a fee shard as a zero-data program-owned account on first use,
/// appending the bump to the caller's seeds for `invoke_signed`. A no-op once the
/// shard exists, so fees can accumulate into a shard before it is ever swept.
///
/// # Parameters
/// - `program_id` — the program that will own the shard.
/// - `payer` — funds shard creation.
/// - `shard_account` — the shard account to create when uninitialized.
/// - `system_program_account` — System Program for the creation CPIs.
/// - `signer_seeds` — the shard's seeds without the bump (the bump is appended).
/// - `bump` — the shard PDA's canonical bump.
/// # Returns
/// - `Ok(())` once the shard exists (created or already present).
pub fn ensure_shard_initialized<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    shard_account: &AccountInfo<'a>,
    system_program_account: &AccountInfo<'a>,
    signer_seeds: &[&[u8]],
    bump: u8,
) -> ProgramResult {
    if !is_uninitialized(shard_account) {
        return Ok(());
    }

    let mut seeds_with_bump: Vec<&[u8]> = signer_seeds.to_vec();
    let bump_slice = &[bump];

    seeds_with_bump.push(bump_slice);

    create_pda_account(
        program_id,
        payer,
        shard_account,
        system_program_account,
        0,
        &seeds_with_bump,
    )
}

/// Asserts the fee-shard account prologue shared by every fee-charging
/// instruction: both shards must be writable and `system_program_account` must
/// be the real System Program, checked in the fixed treasury, author-fee, system
/// order. Bundles only the validation asserts; the explicit `next_account_info`
/// wiring at each call site is unchanged.
///
/// # Parameters
/// - `treasury_shard` — treasury shard account, must be writable.
/// - `author_fee_shard` — author-fee shard account, must be writable.
/// - `system_program_account` — must be the real System Program.
/// # Returns
/// - `Ok(())` if all prologue checks pass, else the first failing assert's error.
pub fn assert_fee_shard_accounts(
    treasury_shard: &AccountInfo,
    author_fee_shard: &AccountInfo,
    system_program_account: &AccountInfo,
) -> ProgramResult {
    assert_writable(treasury_shard)?;
    assert_writable(author_fee_shard)?;
    assert_system_program(system_program_account)?;
    Ok(())
}

/// Validates the treasury then author-fee shard PDAs (in that fixed order) and
/// returns their canonical bumps. Shared by every fee-charging instruction.
///
/// # Parameters
/// - `program_id` — this program's id, for deriving the expected PDAs.
/// - `treasury_shard` — treasury shard account to validate.
/// - `author_fee_shard` — author-fee shard account to validate.
/// - `thread_key` — thread the author-fee shard belongs to.
/// - `treasury_shard_idx` — treasury shard index.
/// - `author_fee_shard_idx` — author-fee shard index.
/// # Returns
/// - `Ok((treasury_bump, author_fee_bump))`, or `InvalidShard`/`InvalidPda`.
pub fn validate_fee_shards(
    program_id: &Pubkey,
    treasury_shard: &AccountInfo,
    author_fee_shard: &AccountInfo,
    thread_key: &Pubkey,
    treasury_shard_idx: u16,
    author_fee_shard_idx: u8,
) -> Result<(u8, u8), ProgramError> {
    let treasury_shard_bump = validate_treasury_shard(program_id, treasury_shard, treasury_shard_idx)?;
    let author_fee_shard_bump =
        validate_author_fee_shard(program_id, thread_key, author_fee_shard, author_fee_shard_idx)?;
    Ok((treasury_shard_bump, author_fee_shard_bump))
}

/// Ensures both fee shards are program-owned, rent-exempt accounts (creating them
/// on first use) in the canonical treasury-then-author order.
///
/// # Parameters
/// - `program_id` — the program that will own the shards.
/// - `payer` — funds any shard creation.
/// - `treasury_shard` — treasury shard account.
/// - `author_fee_shard` — author-fee shard account.
/// - `system_program_account` — System Program for the creation CPIs.
/// - `thread_key` — thread the author-fee shard belongs to (used in its seeds).
/// - `treasury_shard_idx` — treasury shard index (used in its seeds).
/// - `author_fee_shard_idx` — author-fee shard index (used in its seeds).
/// - `treasury_shard_bump` — treasury shard's canonical bump.
/// - `author_fee_shard_bump` — author-fee shard's canonical bump.
/// # Returns
/// - `Ok(())` once both shards exist, else the creation CPI error.
#[allow(clippy::too_many_arguments)]
pub fn ensure_fee_shards_initialized<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    treasury_shard: &AccountInfo<'a>,
    author_fee_shard: &AccountInfo<'a>,
    system_program_account: &AccountInfo<'a>,
    thread_key: &Pubkey,
    treasury_shard_idx: u16,
    author_fee_shard_idx: u8,
    treasury_shard_bump: u8,
    author_fee_shard_bump: u8,
) -> ProgramResult {
    ensure_shard_initialized(
        program_id, payer, treasury_shard, system_program_account,
        &[TREASURY_SHARD_SEED, &treasury_shard_idx.to_le_bytes()], treasury_shard_bump,
    )?;
    ensure_shard_initialized(
        program_id, payer, author_fee_shard, system_program_account,
        &[AUTHOR_FEE_SEED, thread_key.as_ref(), &[author_fee_shard_idx]], author_fee_shard_bump,
    )?;
    Ok(())
}

/// Validates both fee-shard PDAs and ensures they are initialized, program-owned,
/// rent-exempt accounts — the canonical treasury-then-author `validate_fee_shards`
/// then `ensure_fee_shards_initialized` sequence performed back-to-back, keeping
/// the bump plumbing internal. Used by the fee-charging instructions that run the
/// two steps contiguously (`like_content`, `request_access`). Instructions that
/// interleave gating between validation and initialization (`fill_slot`), or that
/// only touch the treasury shard (`append_content`), keep calling the two granular
/// helpers directly.
///
/// # Parameters
/// - `program_id` — this program's id, for validation and ownership.
/// - `payer` — funds any shard creation.
/// - `treasury_shard` — treasury shard account.
/// - `author_fee_shard` — author-fee shard account.
/// - `system_program_account` — System Program for the creation CPIs.
/// - `thread_key` — thread the author-fee shard belongs to.
/// - `treasury_shard_idx` — treasury shard index.
/// - `author_fee_shard_idx` — author-fee shard index.
/// # Returns
/// - `Ok(())` once both shards are validated and initialized, else the first
///   failing step's error.
#[allow(clippy::too_many_arguments)]
pub fn prepare_fee_shards<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    treasury_shard: &AccountInfo<'a>,
    author_fee_shard: &AccountInfo<'a>,
    system_program_account: &AccountInfo<'a>,
    thread_key: &Pubkey,
    treasury_shard_idx: u16,
    author_fee_shard_idx: u8,
) -> ProgramResult {
    let (treasury_shard_bump, author_fee_shard_bump) = validate_fee_shards(
        program_id,
        treasury_shard,
        author_fee_shard,
        thread_key,
        treasury_shard_idx,
        author_fee_shard_idx,
    )?;
    ensure_fee_shards_initialized(
        program_id,
        payer,
        treasury_shard,
        author_fee_shard,
        system_program_account,
        thread_key,
        treasury_shard_idx,
        author_fee_shard_idx,
        treasury_shard_bump,
        author_fee_shard_bump,
    )?;
    Ok(())
}

/// Transfers a fee shard's balance above the rent-exempt minimum to `recipient`,
/// leaving the shard exactly rent-exempt. A no-op when the shard holds no excess.
/// Shared by the treasury and author-fee sweeps, which differ only in PDA
/// validation and recipient.
///
/// # Parameters
/// - `shard_account` — the fee shard being swept.
/// - `recipient` — receives the swept excess lamports.
/// - `shard_rent_min` — rent-exempt minimum to leave behind in the shard.
/// # Returns
/// - `Ok(())` once any excess is moved, else `InvalidAccountData` on overflow.
pub fn sweep_shard_excess(
    shard_account: &AccountInfo,
    recipient: &AccountInfo,
    shard_rent_min: u64,
) -> ProgramResult {
    let excess = shard_account.lamports().saturating_sub(shard_rent_min);
    if excess > 0 {
        **shard_account.lamports.borrow_mut() = shard_rent_min;
        **recipient.lamports.borrow_mut() = recipient
            .lamports()
            .checked_add(excess)
            .ok_or(ProtocolError::InvalidAccountData)?;
    }
    Ok(())
}
