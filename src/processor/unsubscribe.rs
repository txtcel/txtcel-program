use borsh::BorshSerialize;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
    sysvar::{rent::Rent, Sysvar},
};

use crate::error::ProtocolError;
use crate::state::*;

/// Unfollows a channel for the signer.
///
/// Removes the channel address from the caller's `FollowRegistry` (swap-remove,
/// then shrink + refund the freed rent) and decrements the channel's follower
/// counter on the shard derived from the caller's wallet.
///
/// Notes:
/// - The channel account is validated (owner + tag) for symmetry with subscribe;
///   its key is the channel id used to locate the registry entry and shard.
/// - Swap-remove keeps the operation O(1) and avoids shifting the array; the
///   order of the follow list is not significant.
/// - The rent freed by shrinking the registry is refunded back to the owner.
/// - The follower counter shard must exist: a follow always created/bumped it,
///   and the registry entry we just removed proves this wallet had followed the
///   channel.
///
/// Accounts:
/// 0. `[signer, writable]` user - follower + rent-refund recipient.
/// 1. `[writable]` follow_registry - FollowRegistry PDA for `user`.
/// 2. `[writable]` follower_shard - FollowerShard PDA for (thread, user shard).
/// 3. `[]` thread_account - channel being unfollowed (its key is the channel id).
/// 4. `[]` system_program
///
/// # Parameters
/// - `program_id` — this program's address, used for PDA derivation/ownership.
/// - `accounts` — the account list described above, in order.
///
/// # Returns
/// - `Ok(())` once the channel is removed, rent refunded, and counter decremented.
/// - `ProtocolError::Unauthorized`/`NotFollowing`, or PDA/validation errors.
pub fn process_unsubscribe(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let user = next_account_info(account_info_iter)?;
    let follow_registry = next_account_info(account_info_iter)?;
    let follower_shard = next_account_info(account_info_iter)?;
    let thread_account = next_account_info(account_info_iter)?;
    let system_program_account = next_account_info(account_info_iter)?;

    assert_signer(user)?;
    assert_writable(user)?;
    assert_writable(follow_registry)?;
    assert_writable(follower_shard)?;
    assert_system_program(system_program_account)?;
    assert_owned_by(follow_registry, program_id)?;

    let _thread = load_thread(program_id, thread_account)?;
    let thread_key = *thread_account.key;

    let (expected_registry, _) = derive_follow_registry_pda(program_id, user.key);
    assert_pda(follow_registry, &expected_registry)?;

    let shard_idx = follower_shard_index(user.key);
    let (expected_shard, _) = derive_follower_shard_pda(program_id, &thread_key, shard_idx);
    assert_pda(follower_shard, &expected_shard)?;

    let mut registry = load_follow_registry(follow_registry)?;
    if registry.owner != *user.key {
        return Err(ProtocolError::Unauthorized.into());
    }

    let pos = registry
        .channels
        .iter()
        .position(|c| c == &thread_key)
        .ok_or(ProtocolError::NotFollowing)?;

    registry.channels.swap_remove(pos);

    let new_size = FollowRegistry::size(registry.channels.len());
    follow_registry.resize(new_size)?;
    registry.serialize(&mut &mut follow_registry.data.borrow_mut()[..])?;

    let rent = Rent::get()?;
    let required = rent.minimum_balance(new_size);
    let current = follow_registry.lamports();
    if current > required {
        let refund = current - required;
        **follow_registry.lamports.borrow_mut() = required;
        **user.lamports.borrow_mut() = user
            .lamports()
            .checked_add(refund)
            .ok_or(ProtocolError::InvalidAccountData)?;
    }

    if is_uninitialized(follower_shard) {
        return Err(ProtocolError::NotFollowing.into());
    }
    let mut shard = load_bound_follower_shard(program_id, follower_shard, &thread_key, shard_idx)?;
    shard.count = shard.count.saturating_sub(1);
    shard.serialize(&mut &mut follower_shard.data.borrow_mut()[..])?;

    Ok(())
}
