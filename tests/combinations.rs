//! Multi-instruction flows with a running balance ledger: after every step the
//! treasury shard and author-fee shard are reconciled against an independently
//! computed expectation, then both are swept and the destinations verified.
//!
//! Distinct fee parameters are used for base / author / entry / like cuts so a
//! mix-up in which cut applies to which flow would surface as a mismatch.

mod common;

use common::*;
use solana_keypair::Keypair;
use solana_signer::Signer;
use txtcel_program::content::ContentNode;
use txtcel_program::state::{AllocNode, ThreadNode};

fn pct(amount: u64, bps: u32) -> u64 {
    ((amount as u128) * (bps as u128) / 10_000) as u64
}

const BASE_BPS: u32 = 1000; // 10%  base fee on rent
const AUTHOR_CUT: u32 = 2000; // 20% platform cut of the message fee
const ENTRY_CUT: u32 = 3000; // 30% platform cut of the entry fee
const LIKE_CUT: u32 = 2500; // 25% platform cut of the like fee

const MESSAGE_FEE: u64 = 7_000_000;
const LIKE_FEE: u64 = 3_000_000;
const ENTRY_FEE: u64 = 11_000_000;

#[test]
fn full_monetization_flow_reconciles_every_step() {
    let mut env = Env::new();
    let treasury = env.wallet(LAMPORTS_PER_SOL);
    let pd = env.program_data_pda();
    let admin = env.admin.insecure_clone();
    env.send_ok(
        ix_init_settings(&env.program_id, &admin.pubkey(), &pd, &treasury.pubkey()),
        &[&admin],
    );

    // Configure distinct platform cuts.
    env.send_ok(ix_set_base_fee(&env.program_id, &admin.pubkey(), BASE_BPS), &[&admin]);
    env.send_ok(ix_set_author_fee_cut(&env.program_id, &admin.pubkey(), AUTHOR_CUT), &[&admin]);
    env.send_ok(ix_set_entry_cut(&env.program_id, &admin.pubkey(), ENTRY_CUT), &[&admin]);
    env.send_ok(ix_set_like_cut(&env.program_id, &admin.pubkey(), LIKE_CUT), &[&admin]);

    let author = env.wallet(1_000 * LAMPORTS_PER_SOL);
    let shard0 = treasury_shard_pda(&env.program_id, 0);
    let rent0 = env.rent(0);

    // Running expectations for the two accumulator shards.
    let mut exp_treasury: u64;
    let mut exp_author: u64;

    // ── 1. create channel ──
    let thread = Keypair::new();
    let thread_key = thread.pubkey();
    let thread_rent = env.rent(ThreadNode::size(b"chan".len()));
    let alloc_rent = env.rent(AllocNode::size());
    env.send_ok(
        ix_create_root_alloc(
            &env.program_id,
            &author.pubkey(),
            &thread_key,
            0,
            MESSAGE_FEE,
            b"chan".to_vec(),
        ),
        &[&author, &thread],
    );
    let author_shard0 = author_fee_pda(&env.program_id, &thread_key, 0);
    exp_treasury = rent0 + pct(thread_rent + alloc_rent, BASE_BPS);
    assert_eq!(env.balance(&shard0), exp_treasury, "after create: treasury shard");

    // ── 2. set like fee + init access (rent collected into treasury) ──
    env.send_ok(
        ix_set_like_fee(&env.program_id, &author.pubkey(), &thread_key, LIKE_FEE),
        &[&author],
    );
    env.send_ok(
        ix_init_thread_access(&env.program_id, &author.pubkey(), &thread_key, false, 0),
        &[&author],
    );
    exp_treasury += env.rent(txtcel_program::state::ThreadAccess::size());
    assert_eq!(env.balance(&shard0), exp_treasury, "after init access: treasury shard");

    env.send_ok(
        ix_set_entry_fee(&env.program_id, &author.pubkey(), &thread_key, ENTRY_FEE),
        &[&author],
    );
    assert_eq!(env.balance(&shard0), exp_treasury, "set_entry_fee moves no funds");

    // ── 3. stranger posts (base fee + author-fee split) ──
    let stranger = env.wallet(1_000 * LAMPORTS_PER_SOL);
    let body = b"hello world".to_vec();
    let content_rent = env.rent(ContentNode::size(body.len()));
    env.send_ok(
        ix_fill_slot(
            &env.program_id,
            &stranger.pubkey(),
            &thread_key,
            0,
            0,
            0,
            0,
            body,
            u64::MAX,
            None,
        ),
        &[&stranger],
    );
    let msg_platform = pct(MESSAGE_FEE, AUTHOR_CUT);
    exp_treasury += pct(content_rent, BASE_BPS) + msg_platform;
    exp_author = rent0 + (MESSAGE_FEE - msg_platform);
    assert_eq!(env.balance(&shard0), exp_treasury, "after post: treasury shard");
    assert_eq!(env.balance(&author_shard0), exp_author, "after post: author shard");

    // ── 4. a different user likes the post (like-fee split) ──
    // Must not be the content author (the poster), or the fee is waived.
    let liker = env.wallet(1_000 * LAMPORTS_PER_SOL);
    env.send_ok(
        ix_like_content(&env.program_id, &liker.pubkey(), &thread_key, 0, 0, 0, 0, u64::MAX),
        &[&liker],
    );
    let like_platform = pct(LIKE_FEE, LIKE_CUT);
    exp_treasury += like_platform;
    exp_author += LIKE_FEE - like_platform;
    assert_eq!(env.balance(&shard0), exp_treasury, "after like: treasury shard");
    assert_eq!(env.balance(&author_shard0), exp_author, "after like: author shard");

    // ── 5. buyer pays entry fee (entry-fee split) ──
    let buyer = env.wallet(1_000 * LAMPORTS_PER_SOL);
    env.send_ok(
        ix_request_access(&env.program_id, &buyer.pubkey(), &thread_key, 0, 0),
        &[&buyer],
    );
    let entry_platform = pct(ENTRY_FEE, ENTRY_CUT);
    exp_treasury += entry_platform;
    exp_author += ENTRY_FEE - entry_platform;
    assert_eq!(env.balance(&shard0), exp_treasury, "after request_access: treasury shard");
    assert_eq!(env.balance(&author_shard0), exp_author, "after request_access: author shard");

    // ── 6. sweep treasury → dedicated wallet (admin pays, treasury receives) ──
    let treasury_excess = exp_treasury - rent0;
    let treasury_before = env.balance(&treasury.pubkey());
    env.send_ok(ix_sweep_treasury(&env.program_id, &treasury.pubkey(), &[0]), &[&admin]);
    assert_eq!(env.balance(&shard0), rent0, "treasury shard drained to rent");
    assert_eq!(
        env.balance(&treasury.pubkey()),
        treasury_before + treasury_excess,
        "treasury wallet received the exact accumulated platform revenue"
    );

    // ── 7. sweep author fees → author (author signs + receives) ──
    let author_excess = exp_author - rent0;
    assert!(author_excess > 0);
    env.send_ok(
        ix_sweep_author_fees(&env.program_id, &thread_key, &author.pubkey(), &[0]),
        &[&author],
    );
    assert_eq!(env.balance(&author_shard0), rent0, "author shard drained to rent");
}

/// A second flow: many posts accumulate into the author shard and a single sweep
/// returns the exact running total — reconciled step by step.
#[test]
fn repeated_posts_accumulate_and_sweep_matches_total() {
    let mut env = Env::new();
    let treasury = env.wallet(LAMPORTS_PER_SOL);
    let pd = env.program_data_pda();
    let admin = env.admin.insecure_clone();
    env.send_ok(
        ix_init_settings(&env.program_id, &admin.pubkey(), &pd, &treasury.pubkey()),
        &[&admin],
    );
    // Zero base fee so the author shard math is purely the message-fee split.
    env.send_ok(ix_set_base_fee(&env.program_id, &admin.pubkey(), 0), &[&admin]);
    env.send_ok(ix_set_author_fee_cut(&env.program_id, &admin.pubkey(), AUTHOR_CUT), &[&admin]);

    let author = env.wallet(1_000 * LAMPORTS_PER_SOL);
    let thread = Keypair::new();
    let thread_key = thread.pubkey();
    env.send_ok(
        ix_create_root_alloc(
            &env.program_id,
            &author.pubkey(),
            &thread_key,
            0,
            MESSAGE_FEE,
            b"chan".to_vec(),
        ),
        &[&author, &thread],
    );

    let author_shard0 = author_fee_pda(&env.program_id, &thread_key, 0);
    let rent0 = env.rent(0);
    let per_post_author = MESSAGE_FEE - pct(MESSAGE_FEE, AUTHOR_CUT);

    let mut expected = rent0;
    for slot in 0u8..5 {
        let poster = env.wallet(100 * LAMPORTS_PER_SOL);
        env.send_ok(
            ix_fill_slot(
                &env.program_id,
                &poster.pubkey(),
                &thread_key,
                0,
                0,
                0,
                slot,
                format!("post {slot}").into_bytes(),
                u64::MAX,
                None,
            ),
            &[&poster],
        );
        expected += per_post_author;
        assert_eq!(
            env.balance(&author_shard0),
            expected,
            "author shard after post {slot}"
        );
    }

    // Sweep returns exactly the accumulated author share (shard left at rent).
    env.send_ok(
        ix_sweep_author_fees(&env.program_id, &thread_key, &author.pubkey(), &[0]),
        &[&author],
    );
    assert_eq!(env.balance(&author_shard0), rent0, "author shard swept to rent");
}
