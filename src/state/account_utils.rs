//! Low-level account lifecycle helpers: rent top-ups, PDA creation hardened
//! against pre-funding griefing, and uniform account closing.
//!
//! These wrap the System Program CPIs the program performs most often and
//! concentrate the security-sensitive ordering (allocate/assign vs.
//! create_account, zero-then-reassign on close) in one reviewed place.

use solana_program::{
    account_info::AccountInfo,
    entrypoint::ProgramResult,
    program::{invoke, invoke_signed},
    pubkey::Pubkey,
    sysvar::{rent::Rent, Sysvar},
};
use solana_system_interface::{instruction as system_instruction, program as system_program};

use crate::error::ProtocolError;

use super::asserts::assert_system_program;

/// Tops up `target` from `payer` to the rent-exempt minimum for `data_len`,
/// doing nothing when it is already funded. Used after growing a resizable
/// account so it never slips below rent exemption and gets reaped.
///
/// # Parameters
/// - `payer` — funds the top-up transfer.
/// - `target` — the account being brought to rent exemption.
/// - `system_program_account` — System Program, used for the transfer CPI.
/// - `data_len` — the target's new data length, which sets the rent minimum.
/// # Returns
/// - `Ok(())` once the target is rent-exempt (or already was), else the CPI /
///   arithmetic error.
pub fn ensure_rent_exempt<'a>(
    payer: &AccountInfo<'a>,
    target: &AccountInfo<'a>,
    system_program_account: &AccountInfo<'a>,
    data_len: usize,
) -> ProgramResult {
    let rent = Rent::get()?;
    let minimum = rent.minimum_balance(data_len);
    let current = target.lamports();
    if current >= minimum {
        return Ok(());
    }
    let top_up = minimum
        .checked_sub(current)
        .ok_or(ProtocolError::InvalidAccountData)?;
    if top_up > 0 {
        invoke(
            &system_instruction::transfer(payer.key, target.key, top_up),
            &[
                payer.clone(),
                target.clone(),
                system_program_account.clone(),
            ],
        )?;
    }
    Ok(())
}

/// Empties a program-owned account into `recipient`: refunds all lamports, zeroes
/// and shrinks the data, then reassigns ownership to the System Program. Reusing
/// one routine keeps every close path's operation order identical and prevents a
/// freed account from being "revived" with a stale tag in the same transaction.
///
/// # Parameters
/// - `account` — the program-owned account being closed.
/// - `recipient` — receives the closed account's refunded lamports.
/// # Returns
/// - `Ok(())` once emptied and reassigned, else `InvalidAccountData` on lamport
///   overflow or a resize failure.
pub fn close_program_account(account: &AccountInfo, recipient: &AccountInfo) -> ProgramResult {
    let lamports = account.lamports();
    **recipient.lamports.borrow_mut() = recipient
        .lamports()
        .checked_add(lamports)
        .ok_or(ProtocolError::InvalidAccountData)?;
    **account.lamports.borrow_mut() = 0;
    account.data.borrow_mut().fill(0);
    account.resize(0)?;
    account.assign(&system_program::ID);
    Ok(())
}

/// Creates a program-owned PDA account that is robust against the
/// "create_account pre-funding" DoS. The System Program's `create_account`
/// fails if the destination already holds lamports, and PDA addresses are
/// fully predictable, so anyone can permanently brick lazy account creation by
/// sending 1 lamport to the future PDA address ahead of time.
///
/// To avoid that, when the account is already pre-funded we never call
/// `create_account`. Instead we top up the rent (if needed) with a plain
/// transfer, then `allocate` the data and `assign` ownership to the program.
/// An attacker can fund the address but cannot `allocate`/`assign` it (those
/// require the PDA's own signature, which only this program can provide via
/// `invoke_signed`), so the account is guaranteed to have empty, system-owned
/// data and this path always succeeds.
///
/// Notes:
/// - The `system_instruction::*` builders hard-code the System Program as the
///   CPI target, but we additionally assert the passed account is the real
///   System Program for defense-in-depth.
/// - On the pre-funded path we top up to rent-exemption, then take ownership.
///
/// # Parameters
/// - `program_id` — the program that will own the new account.
/// - `payer` — funds account creation / rent top-up.
/// - `target` — the PDA being created (its address must match `signer_seeds`).
/// - `system_program_account` — System Program for the create/allocate/assign CPIs.
/// - `space` — data length to allocate for the account.
/// - `signer_seeds` — the PDA's full seeds including the bump; required so
///   `invoke_signed` can sign as the PDA.
/// # Returns
/// - `Ok(())` once the account is program-owned and rent-exempt, else the
///   underlying CPI error.
pub fn create_pda_account<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    target: &AccountInfo<'a>,
    system_program_account: &AccountInfo<'a>,
    space: usize,
    signer_seeds: &[&[u8]],
) -> ProgramResult {
    assert_system_program(system_program_account)?;

    let rent = Rent::get()?;
    let required = rent.minimum_balance(space);
    let current = target.lamports();

    if current == 0 {
        invoke_signed(
            &system_instruction::create_account(
                payer.key,
                target.key,
                required,
                space as u64,
                program_id,
            ),
            &[
                payer.clone(),
                target.clone(),
                system_program_account.clone(),
            ],
            &[signer_seeds],
        )?;
        return Ok(());
    }

    if current < required {
        let top_up = required - current;
        invoke(
            &system_instruction::transfer(payer.key, target.key, top_up),
            &[
                payer.clone(),
                target.clone(),
                system_program_account.clone(),
            ],
        )?;
    }

    invoke_signed(
        &system_instruction::allocate(target.key, space as u64),
        &[target.clone(), system_program_account.clone()],
        &[signer_seeds],
    )?;

    invoke_signed(
        &system_instruction::assign(target.key, program_id),
        &[target.clone(), system_program_account.clone()],
        &[signer_seeds],
    )?;

    Ok(())
}
