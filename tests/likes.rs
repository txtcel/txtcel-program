//! Likes: counter creation/increment, like-fee splitting, author exemption,
//! slippage cap, and counter reset on content close.

mod common;

use common::*;
use solana_keypair::Keypair;
use solana_signer::Signer;
use txtcel_program::error::ProtocolError;
use txtcel_program::state::AllocLikes;

fn pct(amount: u64, bps: u32) -> u64 {
    ((amount as u128) * (bps as u128) / 10_000) as u64
}

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

/// Channel + one post in slot 0 by the author. Returns the thread keypair.
fn channel_with_post(env: &mut Env, author: &Keypair, like_fee: u64) -> Keypair {
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
    if like_fee > 0 {
        env.send_ok(
            ix_set_like_fee(&env.program_id, &author.pubkey(), &thread.pubkey(), like_fee),
            &[author],
        );
    }
    env.send_ok(
        ix_fill_slot(
            &env.program_id,
            &author.pubkey(),
            &thread.pubkey(),
            0,
            0,
            0,
            0,
            b"likeable".to_vec(),
            u64::MAX,
            None,
        ),
        &[author],
    );
    thread
}

#[test]
fn like_creates_counter_and_increments() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = channel_with_post(&mut env, &author, 0);
    let thread_key = thread.pubkey();

    let liker = env.wallet(100 * LAMPORTS_PER_SOL);
    let likes_key = likes_pda(&env.program_id, &thread_key, 0);

    env.send_ok(
        ix_like_content(&env.program_id, &liker.pubkey(), &thread_key, 0, 0, 0, 0, u64::MAX),
        &[&liker],
    );

    assert_eq!(env.balance(&likes_key), env.rent(AllocLikes::size()), "likes account rent");
    let likes: AllocLikes = env.decode(&likes_key);
    assert_eq!(likes.counts[0], 1, "slot 0 liked once");

    // Second like bumps the counter.
    let liker2 = env.wallet(100 * LAMPORTS_PER_SOL);
    env.send_ok(
        ix_like_content(&env.program_id, &liker2.pubkey(), &thread_key, 0, 0, 0, 0, u64::MAX),
        &[&liker2],
    );
    let likes: AllocLikes = env.decode(&likes_key);
    assert_eq!(likes.counts[0], 2);
}

#[test]
fn like_fee_is_split_between_author_and_treasury() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let like_fee = 4_000_000u64;
    let thread = channel_with_post(&mut env, &author, like_fee);
    let thread_key = thread.pubkey();

    let liker = env.wallet(100 * LAMPORTS_PER_SOL);
    let shard0 = treasury_shard_pda(&env.program_id, 0);
    let author_shard0 = author_fee_pda(&env.program_id, &thread_key, 0);
    let treasury_before = env.balance(&shard0);

    let platform_cut = pct(like_fee, 1000); // like_cut_bps default
    let author_receives = like_fee - platform_cut;

    env.send_ok(
        ix_like_content(&env.program_id, &liker.pubkey(), &thread_key, 0, 0, 0, 0, u64::MAX),
        &[&liker],
    );

    assert_eq!(
        env.balance(&shard0),
        treasury_before + platform_cut,
        "treasury += platform cut of like fee"
    );
    assert_eq!(
        env.balance(&author_shard0),
        env.rent(0) + author_receives,
        "author shard += author's share of like fee"
    );
}

#[test]
fn author_likes_own_content_for_free() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let like_fee = 4_000_000u64;
    let thread = channel_with_post(&mut env, &author, like_fee);
    let thread_key = thread.pubkey();

    let shard0 = treasury_shard_pda(&env.program_id, 0);
    let author_shard0 = author_fee_pda(&env.program_id, &thread_key, 0);
    let treasury_before = env.balance(&shard0);

    env.send_ok(
        ix_like_content(&env.program_id, &author.pubkey(), &thread_key, 0, 0, 0, 0, u64::MAX),
        &[&author],
    );

    assert_eq!(env.balance(&shard0), treasury_before, "no treasury change");
    assert_eq!(env.balance(&author_shard0), env.rent(0), "no author fee (self-like)");
}

#[test]
fn like_respects_max_fee() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = channel_with_post(&mut env, &author, 10_000_000);
    let thread_key = thread.pubkey();

    let liker = env.wallet(100 * LAMPORTS_PER_SOL);
    let res = env.send(
        ix_like_content(&env.program_id, &liker.pubkey(), &thread_key, 0, 0, 0, 0, 0),
        &[&liker],
    );
    assert_custom_error(res, protocol_code(ProtocolError::FeeExceedsMax));
}

#[test]
fn closing_content_resets_its_like_counter() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = channel_with_post(&mut env, &author, 0);
    let thread_key = thread.pubkey();

    let liker = env.wallet(100 * LAMPORTS_PER_SOL);
    env.send_ok(
        ix_like_content(&env.program_id, &liker.pubkey(), &thread_key, 0, 0, 0, 0, u64::MAX),
        &[&liker],
    );
    let likes_key = likes_pda(&env.program_id, &thread_key, 0);
    let content = content_pda(&env.program_id, &thread_key, 0, 0);

    // Author closes the content and passes the likes PDA to reset the slot.
    env.send_ok(
        ix_close_account(&env.program_id, &author.pubkey(), &content, Some(&likes_key)),
        &[&author],
    );

    let likes: AllocLikes = env.decode(&likes_key);
    assert_eq!(likes.counts[0], 0, "slot like counter reset on close");
}
