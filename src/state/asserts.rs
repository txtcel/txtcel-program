//! Generic account-validation guards reused by every instruction.
//!
//! Each helper enforces one precondition (signer, writability, ownership,
//! emptiness, PDA equality, system-program identity, upgrade authority) and
//! returns a protocol error on failure, so processors can compose them into a
//! readable validation prologue instead of repeating the same checks inline.

use solana_program::{
    account_info::AccountInfo,
    entrypoint::ProgramResult,
    program_error::ProgramError,
    pubkey::Pubkey,
};
use solana_system_interface::program as system_program;

use crate::error::ProtocolError;

/// Requires the account to have signed the transaction; the baseline authority
/// check before trusting an account as an actor.
///
/// # Parameters
/// - `account` — the account expected to be a transaction signer.
/// # Returns
/// - `Ok(())` if signed, else `MissingSigner`.
pub fn assert_signer(account: &AccountInfo) -> ProgramResult {
    if account.is_signer {
        Ok(())
    } else {
        Err(ProtocolError::MissingSigner.into())
    }
}

/// Requires the account to be writable, guarding against mutating an account
/// the caller did not mark writable (which would fail late inside a CPI).
///
/// # Parameters
/// - `account` — the account expected to be marked writable.
/// # Returns
/// - `Ok(())` if writable, else `NotWritable`.
pub fn assert_writable(account: &AccountInfo) -> ProgramResult {
    if account.is_writable {
        Ok(())
    } else {
        Err(ProtocolError::NotWritable.into())
    }
}

/// Requires the account to be fresh (system-owned and empty) before the program
/// initializes it, so an existing account is never silently overwritten.
///
/// # Parameters
/// - `account` — the account expected to be uninitialized.
/// # Returns
/// - `Ok(())` if system-owned and empty, else `AccountAlreadyInitialized`.
pub fn assert_uninitialized(account: &AccountInfo) -> ProgramResult {
    if system_program::check_id(account.owner) && account.data_len() == 0 {
        Ok(())
    } else {
        Err(ProtocolError::AccountAlreadyInitialized.into())
    }
}

/// Non-erroring form of `assert_uninitialized`: reports whether an account is
/// still system-owned and empty, used by lazy "create on first use" paths.
///
/// # Parameters
/// - `account` — the account to inspect.
/// # Returns
/// - `true` if system-owned and empty, else `false`.
pub fn is_uninitialized(account: &AccountInfo) -> bool {
    system_program::check_id(account.owner) && account.data_len() == 0
}

// ── system helpers ──

/// Requires the passed account to actually be the System Program, hardening CPIs
/// against a spoofed system-program account.
///
/// # Parameters
/// - `account` — the account expected to be the System Program.
/// # Returns
/// - `Ok(())` if it is the System Program, else `IncorrectProgramId`.
pub fn assert_system_program(account: &AccountInfo) -> ProgramResult {
    if system_program::check_id(account.key) {
        Ok(())
    } else {
        Err(ProgramError::IncorrectProgramId)
    }
}

/// Requires the account to be owned by `owner`; the standard guard that an
/// account is one of this program's (or the expected program's) accounts.
///
/// # Parameters
/// - `account` — the account whose owner is checked.
/// - `owner` — the program/owner key the account must be owned by.
/// # Returns
/// - `Ok(())` if owned by `owner`, else `AccountOwnerMismatch`.
pub fn assert_owned_by(account: &AccountInfo, owner: &Pubkey) -> ProgramResult {
    if account.owner == owner {
        Ok(())
    } else {
        Err(ProtocolError::AccountOwnerMismatch.into())
    }
}

/// Asserts an account's address equals the expected program-derived address,
/// returning `InvalidPda` otherwise. Centralizes the repeated PDA-equality guard.
///
/// # Parameters
/// - `account` — the account whose address is checked.
/// - `expected` — the canonical PDA the account must match.
/// # Returns
/// - `Ok(())` if the addresses are equal, else `InvalidPda`.
pub fn assert_pda(account: &AccountInfo, expected: &Pubkey) -> ProgramResult {
    if account.key != expected {
        return Err(ProtocolError::InvalidPda.into());
    }
    Ok(())
}

/// Reads the leading tag byte of an account without a full deserialize, used for
/// cheap account-type dispatch (e.g. deciding how to close an account).
///
/// # Parameters
/// - `account` — the account whose first (tag) byte is read.
/// # Returns
/// - `Ok(tag)` with the discriminator byte, or `InvalidAccountData` if empty.
pub fn read_tag(account: &AccountInfo) -> Result<u8, ProgramError> {
    let data = account.data.borrow();
    if data.is_empty() {
        return Err(ProtocolError::InvalidAccountData.into());
    }
    Ok(data[0])
}

/// Address of the BPF upgradeable loader, owner of every program's ProgramData
/// account; pinned here so upgrade-authority checks trust the right loader.
const BPF_LOADER_UPGRADEABLE_ID: Pubkey =
    solana_program::pubkey!("BPFLoaderUpgradeab1e11111111111111111111111");

/// Verifies that `authority` is the program's current upgrade authority by
/// reading the upgrade-authority bytes out of the program's ProgramData PDA.
///
/// Notes:
/// - ProgramData must be owned by the upgradeable loader before we trust the
///   upgrade-authority bytes inside it.
///
/// # Parameters
/// - `program_id` — this program's id, used to derive the expected ProgramData PDA.
/// - `programdata_account` — the program's ProgramData account to read the
///   upgrade authority from (validated as the canonical, loader-owned PDA).
/// - `authority` — the account claiming to be the upgrade authority.
/// # Returns
/// - `Ok(())` if `authority` matches the stored upgrade authority, else
///   `Unauthorized`; `InvalidPda`/`AccountOwnerMismatch` if ProgramData is wrong,
///   or `InvalidAccountData` on a malformed buffer.
pub fn assert_upgrade_authority(
    program_id: &Pubkey,
    programdata_account: &AccountInfo,
    authority: &AccountInfo,
) -> ProgramResult {
    let (expected_programdata, _) = Pubkey::find_program_address(
        &[program_id.as_ref()],
        &BPF_LOADER_UPGRADEABLE_ID,
    );
    assert_pda(programdata_account, &expected_programdata)?;

    assert_owned_by(programdata_account, &BPF_LOADER_UPGRADEABLE_ID)?;

    let data = programdata_account.data.borrow();
    if data.len() < 45 || data[12] != 1 {
        return Err(ProtocolError::Unauthorized.into());
    }
    let upgrade_authority = Pubkey::from(
        <[u8; 32]>::try_from(&data[13..45])
            .map_err(|_| ProtocolError::InvalidAccountData)?,
    );
    if *authority.key != upgrade_authority {
        return Err(ProtocolError::Unauthorized.into());
    }
    Ok(())
}
