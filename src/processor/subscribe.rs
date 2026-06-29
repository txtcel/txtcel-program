use borsh::BorshSerialize;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
};

use crate::error::ProtocolError;
use crate::state::*;

/// Follows a channel for the signer.
///
/// Pushes the channel address into the caller's `FollowRegistry` (created on
/// first follow, grown by one slot otherwise) and bumps the channel's
/// follower counter on the shard derived from the caller's wallet.
///
/// No fee is charged; the caller only funds the rent for their own registry
/// growth and, on first use, the counter shard.
///
/// Notes:
/// - The channel account is validated (owner + tag) and its key is used as the
///   channel id.
/// - The `FollowRegistry` is created empty on first follow, then grown by one
///   channel slot and persisted on each subsequent follow.
/// - The follower counter shard is created on the first follower and bumped
///   otherwise.
///
/// Accounts:
/// 0. `[signer, writable]` user - follower + rent payer.
/// 1. `[writable]` follow_registry - FollowRegistry PDA for `user`.
/// 2. `[writable]` follower_shard - FollowerShard PDA for (thread, user shard).
/// 3. `[]` thread_account - channel being followed (validated owner + tag).
/// 4. `[]` system_program
///
/// # Parameters
/// - `program_id` — this program's address, used for PDA derivation/ownership.
/// - `accounts` — the account list described above, in order.
///
/// # Returns
/// - `Ok(())` once the channel is added and the follower counter bumped.
/// - `ProtocolError::AlreadyFollowing`/`FollowListFull`/`Unauthorized`, or
///   PDA/account-creation errors.
pub fn process_subscribe(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
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

    let _thread = load_thread(program_id, thread_account)?;
    let thread_key = *thread_account.key;

    let (expected_registry, registry_bump) = derive_follow_registry_pda(program_id, user.key);
    assert_pda(follow_registry, &expected_registry)?;

    let shard_idx = follower_shard_index(user.key);
    let (expected_shard, shard_bump) = derive_follower_shard_pda(program_id, &thread_key, shard_idx);
    assert_pda(follower_shard, &expected_shard)?;

    let mut registry = if is_uninitialized(follow_registry) {
        create_pda_account(
            program_id,
            user,
            follow_registry,
            system_program_account,
            FollowRegistry::size(0),
            &[FOLLOWS_SEED, user.key.as_ref(), &[registry_bump]],
        )?;
        let registry = FollowRegistry {
            tag: TAG_FOLLOW_REGISTRY,
            owner: *user.key,
            channels: Vec::new(),
        };
        registry.serialize(&mut &mut follow_registry.data.borrow_mut()[..])?;
        registry
    } else {
        assert_owned_by(follow_registry, program_id)?;
        let registry = load_follow_registry(follow_registry)?;
        if registry.owner != *user.key {
            return Err(ProtocolError::Unauthorized.into());
        }
        registry
    };

    if registry.channels.iter().any(|c| c == &thread_key) {
        return Err(ProtocolError::AlreadyFollowing.into());
    }
    if registry.channels.len() >= MAX_FOLLOWS {
        return Err(ProtocolError::FollowListFull.into());
    }

    registry.channels.push(thread_key);
    let new_size = FollowRegistry::size(registry.channels.len());
    follow_registry.resize(new_size)?;
    ensure_rent_exempt(user, follow_registry, system_program_account, new_size)?;
    registry.serialize(&mut &mut follow_registry.data.borrow_mut()[..])?;

    if is_uninitialized(follower_shard) {
        create_pda_account(
            program_id,
            user,
            follower_shard,
            system_program_account,
            FollowerShard::size(),
            &[
                FOLLOWER_COUNT_SEED,
                thread_key.as_ref(),
                &[shard_idx],
                &[shard_bump],
            ],
        )?;
        let shard = FollowerShard {
            tag: TAG_FOLLOWER_SHARD,
            thread: thread_key,
            shard: shard_idx,
            count: 1,
        };
        shard.serialize(&mut &mut follower_shard.data.borrow_mut()[..])?;
    } else {
        let mut shard = load_bound_follower_shard(program_id, follower_shard, &thread_key, shard_idx)?;
        shard.count = shard
            .count
            .checked_add(1)
            .ok_or(ProtocolError::InvalidAccountData)?;
        shard.serialize(&mut &mut follower_shard.data.borrow_mut()[..])?;
    }

    Ok(())
}
