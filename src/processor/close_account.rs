use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
};
use solana_system_interface::program as system_program;

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
/// - Sets the target account lamports to zero and clears its data.
///
/// Errors:
/// - `InvalidTag` if the account type is not supported for closure.
/// - `Unauthorized` if the payer is not the content author.
/// - `InvalidPda` if the supplied likes account is not the expected PDA.
/// - `InvalidAccountData` on arithmetic overflow.
/// - Any validation error from ownership or signer checks.
pub fn process_close_account(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let payer = next_account_info(account_info_iter)?;
    let target_account = next_account_info(account_info_iter)?;
    // Optional: the AllocLikes account for this content's alloc.
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

            // Clear this slot's like counter so a message later reusing the
            // (alloc_seq, slot) PDA starts with zero likes instead of
            // inheriting the deleted message's count.
            if let Some(likes_account) = likes_account {
                let (expected_likes, _) =
                    derive_likes_pda(program_id, &content.header.thread, content.header.alloc_seq);
                if *likes_account.key != expected_likes {
                    return Err(ProtocolError::InvalidPda.into());
                }

                if !is_uninitialized(likes_account) {
                    assert_writable(likes_account)?;
                    assert_owned_by(likes_account, program_id)?;

                    let mut likes = {
                        let data = likes_account.data.borrow();
                        AllocLikes::try_from_slice(&data)
                            .map_err(|_| ProtocolError::InvalidAccountData)?
                    };
                    if likes.tag != TAG_LIKES {
                        return Err(ProtocolError::InvalidTag.into());
                    }
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

    let target_lamports = target_account.lamports();

    **payer.lamports.borrow_mut() = payer
        .lamports()
        .checked_add(target_lamports)
        .ok_or(ProtocolError::InvalidAccountData)?;

    **target_account.lamports.borrow_mut() = 0;

    // Zero, shrink and reassign to the System Program so the emptied account
    // cannot be "revived" as a program-owned account with a stale tag within
    // the same transaction.
    target_account.data.borrow_mut().fill(0);
    target_account.resize(0)?;
    target_account.assign(&system_program::ID);

    Ok(())
}
