use borsh::BorshSerialize;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
};

use crate::error::ProtocolError;
use crate::state::*;

/// Closes a program-owned account and transfers its lamports to the signer.
///
/// Accounts:
/// 0. `[signer]` payer - The authority requesting account closure.
/// 1. `[writable]` target_account - The program-owned account to close.
/// 2. `[writable, optional]` likes_account - The `AllocLikes` PDA for the
///    content's alloc. When supplied, the like counter for the freed slot is
///    reset to zero so a future message reusing the slot does not inherit the
///    deleted message's likes.
///
/// Behavior:
/// - Ensures the payer is a signer.
/// - Ensures the target account is writable and owned by this program.
/// - Reads the account tag to determine its type.
/// - Currently supports closing only `TAG_CONTENT` accounts.
/// - Verifies that the payer is the author of the content.
/// - Resets the slot's like counter when the likes account is provided.
/// - Transfers all lamports from the target account to the payer.
/// - Sets the target account lamports to zero, then zeroes, shrinks and
///   reassigns it to the System Program so the emptied account cannot be
///   "revived" as a program-owned account with a stale tag within the same
///   transaction.
///
/// Errors:
/// - `InvalidTag` if the account type is not supported for closure.
/// - `Unauthorized` if the payer is not the content author.
/// - `InvalidPda` if the supplied likes account is not the expected PDA.
/// - `InvalidAccountData` on arithmetic overflow.
/// - Any validation error from ownership or signer checks.
///
/// # Parameters
/// - `program_id` — this program's address, used for PDA derivation/ownership.
/// - `accounts` — `[payer(signer), target, likes(optional)]` as described above.
///
/// # Returns
/// - `Ok(())` once the account is drained and reassigned to the System Program.
/// - The error conditions listed above.
pub fn process_close_account(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let payer = next_account_info(account_info_iter)?;
    let target_account = next_account_info(account_info_iter)?;
    let likes_account = account_info_iter.next();

    assert_signer(payer)?;
    assert_writable(target_account)?;
    assert_owned_by(target_account, program_id)?;

    let tag = read_tag(target_account)?;

    match tag {
        TAG_CONTENT => {
            let content = load_content(program_id, target_account)?;
            if content.header.author != *payer.key {
                return Err(ProtocolError::Unauthorized.into());
            }

            if let Some(likes_account) = likes_account {
                let (expected_likes, _) =
                    derive_likes_pda(program_id, &content.header.thread, content.header.alloc_seq);
                assert_pda(likes_account, &expected_likes)?;

                if !is_uninitialized(likes_account) {
                    assert_writable(likes_account)?;
                    assert_owned_by(likes_account, program_id)?;

                    let mut likes = load_alloc_likes(likes_account)?;
                    if likes.alloc_seq != content.header.alloc_seq {
                        return Err(ProtocolError::InvalidAllocSeq.into());
                    }

                    likes.counts[content.header.slot as usize] = 0;
                    likes.serialize(&mut &mut likes_account.data.borrow_mut()[..])?;
                }
            }
        }
        _ => {
            return Err(ProtocolError::InvalidTag.into());
        }
    }

    close_program_account(target_account, payer)?;

    Ok(())
}
