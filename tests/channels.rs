//! Channel structure: alloc chaining (prepare / auto-extend), content appends
//! with the time window, and content closing.

mod common;

use common::*;
use solana_keypair::Keypair;
use solana_signer::Signer;
use txtcel_program::content::ContentNode;
use txtcel_program::error::ProtocolError;
use txtcel_program::state::{AllocNode, ThreadNode};

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

fn create_channel(env: &mut Env, author: &Keypair, message_fee: u64) -> Keypair {
    let thread = Keypair::new();
    env.send_ok(
        ix_create_root_alloc(
            &env.program_id,
            &author.pubkey(),
            &thread.pubkey(),
            0,
            message_fee,
            b"chan".to_vec(),
        ),
        &[author, &thread],
    );
    thread
}

#[test]
fn prepare_alloc_links_new_node() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();

    let alloc1 = alloc_pda(&env.program_id, &thread_key, 1);
    env.send_ok(
        ix_prepare_alloc(&env.program_id, &author.pubkey(), &thread_key, 0),
        &[&author],
    );

    assert_eq!(env.balance(&alloc1), env.rent(AllocNode::size()), "alloc1 rent");
    let a1: AllocNode = env.decode(&alloc1);
    assert_eq!(a1.alloc_seq, 1);

    // The chain is dense and PDA-addressed; seq 0 still exists at its PDA.
    let a0: AllocNode = env.decode(&alloc_pda(&env.program_id, &thread_key, 0));
    assert_eq!(a0.alloc_seq, 0, "alloc0 still present at its PDA");

    let t: ThreadNode = env.decode(&thread_key);
    assert_eq!(t.alloc_count, 2);
    assert_eq!(t.last_alloc_seq, 1);
}

#[test]
fn prepare_alloc_rejects_double_link() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();

    env.send_ok(
        ix_prepare_alloc(&env.program_id, &author.pubkey(), &thread_key, 0),
        &[&author],
    );
    // alloc0 is no longer the tail (last_alloc_seq is now 1), so extending it
    // again is rejected as a non-tail extend.
    let res = env.send(
        ix_prepare_alloc(&env.program_id, &author.pubkey(), &thread_key, 0),
        &[&author],
    );
    assert_custom_error(res, protocol_code(ProtocolError::InvalidAllocSeq));
}

#[test]
fn fill_slot_does_not_touch_chain() {
    // `fill_slot` is now element-only: posting content must never grow the alloc
    // chain, even if the caller appends extra accounts. Linking is exclusively
    // `prepare_alloc`'s job (see `fill_then_prepare_alloc_links_chain`).
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();

    env.send_ok(
        ix_fill_slot(
            &env.program_id,
            &author.pubkey(),
            &thread_key,
            0,
            0,
            0,
            0,
            b"first".to_vec(),
            u64::MAX,
            None,
        ),
        &[&author],
    );

    let alloc1 = alloc_pda(&env.program_id, &thread_key, 1);
    assert_eq!(env.balance(&alloc1), 0, "fill_slot must not create alloc1");
    let t: ThreadNode = env.decode(&thread_key);
    assert_eq!(t.last_alloc_seq, 0, "chain unchanged by fill_slot");
    assert_eq!(t.alloc_count, 1);
}

#[test]
fn fill_then_prepare_alloc_links_chain() {
    // The separated flow: post a content element, then link the next page in a
    // distinct instruction.
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();

    env.send_ok(
        ix_fill_slot(
            &env.program_id,
            &author.pubkey(),
            &thread_key,
            0,
            0,
            0,
            0,
            b"first".to_vec(),
            u64::MAX,
            None,
        ),
        &[&author],
    );

    env.send_ok(
        ix_prepare_alloc(&env.program_id, &author.pubkey(), &thread_key, 0),
        &[&author],
    );

    let alloc1 = alloc_pda(&env.program_id, &thread_key, 1);
    assert_eq!(env.balance(&alloc1), env.rent(AllocNode::size()), "prepare_alloc created alloc1");
    let t: ThreadNode = env.decode(&thread_key);
    assert_eq!(t.last_alloc_seq, 1);
    assert_eq!(t.alloc_count, 2);
}

#[test]
fn append_content_charges_base_fee_on_growth() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();

    // Author posts "hello" in slot 0 (author is fee-exempt; base fee applies).
    env.send_ok(
        ix_fill_slot(
            &env.program_id,
            &author.pubkey(),
            &thread_key,
            0,
            0,
            0,
            0,
            b"hello".to_vec(),
            u64::MAX,
            None,
        ),
        &[&author],
    );

    let content = content_pda(&env.program_id, &thread_key, 0, 0);
    let shard0 = treasury_shard_pda(&env.program_id, 0);
    let old_size = ContentNode::size(5);
    let new_size = ContentNode::size(5 + 6);
    let rent_delta = env.rent(new_size) - env.rent(old_size);
    let expected_fee = pct(rent_delta, 1000);
    let treasury_before = env.balance(&shard0);

    env.send_ok(
        ix_append_content(
            &env.program_id,
            &author.pubkey(),
            &thread_key,
            0,
            0,
            0,
            0,
            b" world".to_vec(),
        ),
        &[&author],
    );

    assert_eq!(env.balance(&content), env.rent(new_size), "content topped up to new rent");
    assert_eq!(
        env.balance(&shard0),
        treasury_before + expected_fee,
        "treasury += base fee on rent delta"
    );
    let node: ContentNode = env.decode(&content);
    assert_eq!(node.body, b"hello world".to_vec());
}

#[test]
fn append_content_rejects_after_window() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();

    env.send_ok(
        ix_fill_slot(
            &env.program_id,
            &author.pubkey(),
            &thread_key,
            0,
            0,
            0,
            0,
            b"hello".to_vec(),
            u64::MAX,
            None,
        ),
        &[&author],
    );

    // Past the 120s append window.
    env.advance_unix_time(200);

    let res = env.send(
        ix_append_content(
            &env.program_id,
            &author.pubkey(),
            &thread_key,
            0,
            0,
            0,
            0,
            b"!".to_vec(),
        ),
        &[&author],
    );
    assert_custom_error(res, protocol_code(ProtocolError::AppendWindowExpired));
}

#[test]
fn append_content_rejects_non_author() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();

    env.send_ok(
        ix_fill_slot(
            &env.program_id,
            &author.pubkey(),
            &thread_key,
            0,
            0,
            0,
            0,
            b"hello".to_vec(),
            u64::MAX,
            None,
        ),
        &[&author],
    );

    let stranger = env.wallet(10 * LAMPORTS_PER_SOL);
    let res = env.send(
        ix_append_content(
            &env.program_id,
            &stranger.pubkey(),
            &thread_key,
            0,
            0,
            0,
            0,
            b"x".to_vec(),
        ),
        &[&stranger],
    );
    assert_custom_error(res, protocol_code(ProtocolError::Unauthorized));
}

#[test]
fn close_account_refunds_content_rent() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();

    env.send_ok(
        ix_fill_slot(
            &env.program_id,
            &author.pubkey(),
            &thread_key,
            0,
            0,
            0,
            0,
            b"deleteme".to_vec(),
            u64::MAX,
            None,
        ),
        &[&author],
    );

    let content = content_pda(&env.program_id, &thread_key, 0, 0);
    assert!(env.balance(&content) > 0, "content exists");
    let author_before = env.balance(&author.pubkey());

    env.send_ok(
        ix_close_account(&env.program_id, &author.pubkey(), &content, None),
        &[&author],
    );

    // Account emptied and returned to the system program.
    let acct = env.account(&content);
    let emptied = acct.map(|a| a.lamports == 0 && a.data.is_empty()).unwrap_or(true);
    assert!(emptied, "content account closed");
    assert!(env.balance(&author.pubkey()) > author_before, "rent refunded to author");
}

#[test]
fn close_account_rejects_non_author() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();

    env.send_ok(
        ix_fill_slot(
            &env.program_id,
            &author.pubkey(),
            &thread_key,
            0,
            0,
            0,
            0,
            b"mine".to_vec(),
            u64::MAX,
            None,
        ),
        &[&author],
    );

    let content = content_pda(&env.program_id, &thread_key, 0, 0);
    let stranger = env.wallet(10 * LAMPORTS_PER_SOL);
    let res = env.send(
        ix_close_account(&env.program_id, &stranger.pubkey(), &content, None),
        &[&stranger],
    );
    assert_custom_error(res, protocol_code(ProtocolError::Unauthorized));
}
