//! End-to-end lifecycle with per-step balance reconciliation.
//!
//! Recipient/account balances (shards, created PDAs, treasury wallet, rent
//! refunds) are asserted exactly via the runtime's own rent calculation, so the
//! checks are independent of the transaction fee. The fee payer is only
//! lower-bounded where it is also a recipient.

mod common;

use common::*;
use solana_signer::Signer;
use txtcel_program::content::ContentNode;
use txtcel_program::state::{
    AllocNode, FollowRegistry, FollowerShard, ProgramSettings, ThreadNode,
};

/// `amount * bps / 10_000`, matching the program's fee math.
fn pct(amount: u64, bps: u32) -> u64 {
    ((amount as u128) * (bps as u128) / 10_000) as u64
}

fn init(env: &mut Env, treasury: &Pk) {
    let pd = env.program_data_pda();
    let admin = env.admin.insecure_clone();
    let ix = ix_init_settings(&env.program_id, &admin.pubkey(), &pd, treasury);
    env.send_ok(ix, &[&admin]);
}

/// Creates a channel and returns its thread keypair.
fn create_channel(env: &mut Env, author: &solana_keypair::Keypair, message_fee: u64) -> solana_keypair::Keypair {
    let thread = solana_keypair::Keypair::new();
    let ix = ix_create_root_alloc(
        &env.program_id,
        &author.pubkey(),
        &thread.pubkey(),
        0,
        message_fee,
        b"chan".to_vec(),
    );
    env.send_ok(ix, &[author, &thread]);
    thread
}

#[test]
fn init_settings_creates_rent_exempt_account() {
    let mut env = Env::new();
    let treasury = env.wallet(0);
    init(&mut env, &treasury.pubkey());

    let settings_key = settings_pda(&env.program_id);
    let acct = env.account(&settings_key).expect("settings exists");
    assert_eq!(acct.owner, env.program_id, "settings owned by program");
    assert_eq!(
        acct.lamports,
        env.rent(ProgramSettings::size()),
        "settings rent-exempt at exact minimum"
    );

    let settings: ProgramSettings = env.decode(&settings_key);
    assert_eq!(settings.admin.to_bytes(), env.admin.pubkey().to_bytes());
    assert_eq!(settings.treasury.to_bytes(), treasury.pubkey().to_bytes());
    assert_eq!(settings.base_fee_bps, 1000);
    assert_eq!(settings.author_fee_cut_bps, 1000);
}

#[test]
fn create_root_alloc_collects_base_fee_to_treasury_shard() {
    let mut env = Env::new();
    let treasury = env.wallet(0);
    init(&mut env, &treasury.pubkey());

    let author = env.wallet(100 * LAMPORTS_PER_SOL);
    let thread = solana_keypair::Keypair::new();

    let thread_rent = env.rent(ThreadNode::size(b"chan".len()));
    let alloc_rent = env.rent(AllocNode::size());
    let shard_rent0 = env.rent(0);
    let expected_base_fee = pct(thread_rent + alloc_rent, 1000);

    let ix = ix_create_root_alloc(
        &env.program_id,
        &author.pubkey(),
        &thread.pubkey(),
        0,
        0,
        b"chan".to_vec(),
    );
    env.send_ok(ix, &[&author, &thread]);

    // Created accounts hold exactly their rent minimum.
    assert_eq!(env.balance(&thread.pubkey()), thread_rent, "thread rent");
    let alloc0 = alloc_pda(&env.program_id, &thread.pubkey(), 0);
    assert_eq!(env.balance(&alloc0), alloc_rent, "alloc rent");

    // Treasury shard 0 = its own rent + the collected base fee.
    let shard0 = treasury_shard_pda(&env.program_id, 0);
    assert_eq!(
        env.balance(&shard0),
        shard_rent0 + expected_base_fee,
        "treasury shard = rent + base fee"
    );

    // Thread state.
    let thread_state: ThreadNode = env.decode(&thread.pubkey());
    assert_eq!(thread_state.author.to_bytes(), author.pubkey().to_bytes());
    assert_eq!(thread_state.alloc_count, 1);
    assert_eq!(thread_state.last_alloc_seq, 0);
}

#[test]
fn fill_slot_author_is_fee_exempt_stranger_pays_author_fee() {
    let mut env = Env::new();
    let treasury = env.wallet(0);
    init(&mut env, &treasury.pubkey());

    let author = env.wallet(100 * LAMPORTS_PER_SOL);
    let message_fee = 1_000_000u64;
    let thread = create_channel(&mut env, &author, message_fee);
    let thread_key = thread.pubkey();

    let shard0 = treasury_shard_pda(&env.program_id, 0);
    let author_shard0 = author_fee_pda(&env.program_id, &thread_key, 0);
    let shard_rent0 = env.rent(0);

    // ── author posts slot 0: fee-exempt, only the base fee on content rent ──
    let treasury_before = env.balance(&shard0);
    let body0 = b"hello".to_vec();
    let content_rent0 = env.rent(ContentNode::size(body0.len()));
    let base_fee0 = pct(content_rent0, 1000);

    let ix = ix_fill_slot(
        &env.program_id,
        &author.pubkey(),
        &thread_key,
        0,
        0,
        0,
        0,
        body0,
        u64::MAX,
        None,
    );
    env.send_ok(ix, &[&author]);

    let content00 = content_pda(&env.program_id, &thread_key, 0, 0);
    assert_eq!(env.balance(&content00), content_rent0, "content rent");
    assert_eq!(
        env.balance(&shard0),
        treasury_before + base_fee0,
        "treasury += base fee (author exempt from author fee)"
    );
    // Author fee shard was created but received no author fee.
    assert_eq!(env.balance(&author_shard0), shard_rent0, "author shard only rent");

    // ── stranger posts slot 1: pays message_fee, split author/platform ──
    let stranger = env.wallet(100 * LAMPORTS_PER_SOL);
    let treasury_before2 = env.balance(&shard0);
    let author_shard_before = env.balance(&author_shard0);
    let body1 = b"hi from stranger".to_vec();
    let content_rent1 = env.rent(ContentNode::size(body1.len()));
    let base_fee1 = pct(content_rent1, 1000);
    let platform_cut = pct(message_fee, 1000); // author_fee_cut_bps default
    let author_receives = message_fee - platform_cut;

    let ix = ix_fill_slot(
        &env.program_id,
        &stranger.pubkey(),
        &thread_key,
        0,
        0,
        0,
        1,
        body1,
        u64::MAX,
        None,
    );
    env.send_ok(ix, &[&stranger]);

    assert_eq!(
        env.balance(&shard0),
        treasury_before2 + base_fee1 + platform_cut,
        "treasury += base fee + platform cut of author fee"
    );
    assert_eq!(
        env.balance(&author_shard0),
        author_shard_before + author_receives,
        "author shard += author's share of message fee"
    );
}

#[test]
fn fill_slot_respects_max_fee_slippage_cap() {
    let mut env = Env::new();
    let treasury = env.wallet(0);
    init(&mut env, &treasury.pubkey());

    let author = env.wallet(100 * LAMPORTS_PER_SOL);
    let thread = create_channel(&mut env, &author, 5_000_000);
    let stranger = env.wallet(100 * LAMPORTS_PER_SOL);

    // max_fee = 0 is below the author fee, so the post must be rejected.
    let ix = ix_fill_slot(
        &env.program_id,
        &stranger.pubkey(),
        &thread.pubkey(),
        0,
        0,
        0,
        0,
        b"blocked".to_vec(),
        0,
        None,
    );
    let res = env.send(ix, &[&stranger]);
    assert_custom_error(res, protocol_code(txtcel_program::error::ProtocolError::FeeExceedsMax));
}

#[test]
fn sweeps_move_excess_to_treasury_and_author() {
    let mut env = Env::new();
    // Treasury is a real, rent-exempt wallet (the sweep tops it up further).
    let treasury = env.wallet(LAMPORTS_PER_SOL);
    init(&mut env, &treasury.pubkey());

    let author = env.wallet(100 * LAMPORTS_PER_SOL);
    let message_fee = 2_000_000u64;
    let thread = create_channel(&mut env, &author, message_fee);
    let thread_key = thread.pubkey();
    let stranger = env.wallet(100 * LAMPORTS_PER_SOL);

    // One stranger post funds both shards.
    let ix = ix_fill_slot(
        &env.program_id,
        &stranger.pubkey(),
        &thread_key,
        0,
        0,
        0,
        0,
        b"msg".to_vec(),
        u64::MAX,
        None,
    );
    env.send_ok(ix, &[&stranger]);

    let shard0 = treasury_shard_pda(&env.program_id, 0);
    let author_shard0 = author_fee_pda(&env.program_id, &thread_key, 0);
    let shard_rent0 = env.rent(0);

    // ── sweep treasury (permissionless; admin pays the tx, treasury receives) ──
    let treasury_excess = env.balance(&shard0) - shard_rent0;
    let treasury_before = env.balance(&treasury.pubkey());
    let admin = env.admin.insecure_clone();
    let ix = ix_sweep_treasury(&env.program_id, &treasury.pubkey(), &[0]);
    env.send_ok(ix, &[&admin]);

    assert_eq!(env.balance(&shard0), shard_rent0, "treasury shard left at rent");
    assert_eq!(
        env.balance(&treasury.pubkey()),
        treasury_before + treasury_excess,
        "treasury wallet received exact excess"
    );

    // ── sweep author fees (author signs + receives) ──
    let author_excess = env.balance(&author_shard0) - shard_rent0;
    assert!(author_excess > 0, "author shard should hold fees");
    let ix = ix_sweep_author_fees(&env.program_id, &thread_key, &author.pubkey(), &[0]);
    env.send_ok(ix, &[&author]);
    assert_eq!(
        env.balance(&author_shard0),
        shard_rent0,
        "author shard left at rent (excess swept out)"
    );
}

#[test]
fn subscribe_then_unsubscribe_updates_registry_and_counter() {
    let mut env = Env::new();
    let treasury = env.wallet(0);
    init(&mut env, &treasury.pubkey());

    let author = env.wallet(100 * LAMPORTS_PER_SOL);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();

    let user = env.wallet(100 * LAMPORTS_PER_SOL);
    let registry_key = follow_registry_pda(&env.program_id, &user.pubkey());
    let shard_idx = follower_shard_index(&user.pubkey());
    let shard_key = follower_shard_pda(&env.program_id, &thread_key, shard_idx);

    // ── subscribe ──
    let ix = ix_subscribe(&env.program_id, &user.pubkey(), &thread_key);
    env.send_ok(ix, &[&user]);

    let registry: FollowRegistry = env.decode(&registry_key);
    assert_eq!(registry.owner.to_bytes(), user.pubkey().to_bytes());
    assert_eq!(registry.channels.len(), 1);
    assert_eq!(registry.channels[0].to_bytes(), thread_key.to_bytes());

    let shard: FollowerShard = env.decode(&shard_key);
    assert_eq!(shard.count, 1);
    assert_eq!(shard.shard, shard_idx);

    // double subscribe is rejected
    let ix = ix_subscribe(&env.program_id, &user.pubkey(), &thread_key);
    let res = env.send(ix, &[&user]);
    assert_custom_error(res, protocol_code(txtcel_program::error::ProtocolError::AlreadyFollowing));

    // ── unsubscribe ──
    let ix = ix_unsubscribe(&env.program_id, &user.pubkey(), &thread_key);
    env.send_ok(ix, &[&user]);

    let registry: FollowRegistry = env.decode(&registry_key);
    assert_eq!(registry.channels.len(), 0, "channel removed from registry");
    let shard: FollowerShard = env.decode(&shard_key);
    assert_eq!(shard.count, 0, "follower counter decremented");

    // unsubscribe again -> not following
    let ix = ix_unsubscribe(&env.program_id, &user.pubkey(), &thread_key);
    let res = env.send(ix, &[&user]);
    assert_custom_error(res, protocol_code(txtcel_program::error::ProtocolError::NotFollowing));
}
