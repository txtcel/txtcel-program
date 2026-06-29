//! Follow registry & sharded follower counters: multi-channel follows, registry
//! rent growth/refund, counter increment/decrement, and validation.

mod common;

use common::*;
use solana_keypair::Keypair;
use solana_signer::Signer;
use txtcel_program::error::ProtocolError;
use txtcel_program::state::{FollowRegistry, FollowerShard};

fn setup(env: &mut Env) -> Keypair {
    let treasury = env.wallet(0);
    let pd = env.program_data_pda();
    let admin = env.admin.insecure_clone();
    env.send_ok(
        ix_init_settings(&env.program_id, &admin.pubkey(), &pd, &treasury.pubkey()),
        &[&admin],
    );
    env.wallet(1_000 * LAMPORTS_PER_SOL)
}

fn create_channel(env: &mut Env, author: &Keypair) -> Keypair {
    let thread = Keypair::new();
    env.send_ok(
        ix_create_root_alloc(
            &env.program_id,
            &author.pubkey(),
            &thread.pubkey(),
            0,
            0,
            b"chan".to_vec(),
        ),
        &[author, &thread],
    );
    thread
}

#[test]
fn subscribe_to_multiple_channels_grows_registry() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let t1 = create_channel(&mut env, &author).pubkey();
    let t2 = create_channel(&mut env, &author).pubkey();

    let user = env.wallet(100 * LAMPORTS_PER_SOL);
    let registry_key = follow_registry_pda(&env.program_id, &user.pubkey());

    env.send_ok(ix_subscribe(&env.program_id, &user.pubkey(), &t1), &[&user]);
    assert_eq!(env.balance(&registry_key), env.rent(FollowRegistry::size(1)));

    env.send_ok(ix_subscribe(&env.program_id, &user.pubkey(), &t2), &[&user]);
    assert_eq!(env.balance(&registry_key), env.rent(FollowRegistry::size(2)));

    let registry: FollowRegistry = env.decode(&registry_key);
    assert_eq!(registry.channels.len(), 2);
    let chans: Vec<[u8; 32]> = registry.channels.iter().map(|c| c.to_bytes()).collect();
    assert!(chans.contains(&t1.to_bytes()));
    assert!(chans.contains(&t2.to_bytes()));

    // Each channel's follower shard for this user counts exactly one.
    let shard_idx = follower_shard_index(&user.pubkey());
    let s1: FollowerShard = env.decode(&follower_shard_pda(&env.program_id, &t1, shard_idx));
    let s2: FollowerShard = env.decode(&follower_shard_pda(&env.program_id, &t2, shard_idx));
    assert_eq!(s1.count, 1);
    assert_eq!(s2.count, 1);
}

#[test]
fn unsubscribe_refunds_registry_rent_delta() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let t1 = create_channel(&mut env, &author).pubkey();
    let t2 = create_channel(&mut env, &author).pubkey();

    let user = env.wallet(100 * LAMPORTS_PER_SOL);
    let registry_key = follow_registry_pda(&env.program_id, &user.pubkey());

    env.send_ok(ix_subscribe(&env.program_id, &user.pubkey(), &t1), &[&user]);
    env.send_ok(ix_subscribe(&env.program_id, &user.pubkey(), &t2), &[&user]);
    assert_eq!(env.balance(&registry_key), env.rent(FollowRegistry::size(2)));

    // Unsubscribe one: registry shrinks back to a single-channel rent.
    env.send_ok(ix_unsubscribe(&env.program_id, &user.pubkey(), &t1), &[&user]);
    assert_eq!(
        env.balance(&registry_key),
        env.rent(FollowRegistry::size(1)),
        "registry shrunk to one channel"
    );
    let registry: FollowRegistry = env.decode(&registry_key);
    assert_eq!(registry.channels.len(), 1);
    assert_eq!(registry.channels[0].to_bytes(), t2.to_bytes());
}

#[test]
fn subscribe_rejects_unknown_thread() {
    let mut env = Env::new();
    let _author = setup(&mut env);
    let user = env.wallet(100 * LAMPORTS_PER_SOL);

    // A pubkey that is not a program-owned thread account.
    let fake_thread = Keypair::new().pubkey();
    let res = env.send(ix_subscribe(&env.program_id, &user.pubkey(), &fake_thread), &[&user]);
    assert_custom_error(res, protocol_code(ProtocolError::AccountOwnerMismatch));
}
