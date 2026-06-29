//! Cases ported from the client integration suite that were missing at the
//! `cargo test` level: multi-candidate slot selection, reply pointers, chunked
//! appends, entry-fee gating, fee-list status guards, and follower aggregation.

mod common;

use common::*;
use solana_keypair::Keypair;
use solana_signer::Signer;
use txtcel_program::content::ContentNode;
use txtcel_program::error::ProtocolError;

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

/// Author posts `text` into slot 0 of a fresh single-candidate fill.
fn author_post(env: &mut Env, author: &Keypair, thread: &Pk, slot: u8, text: &str) {
    env.send_ok(
        ix_fill_slot_ex(
            &env.program_id,
            FillArgs {
                payer: author.pubkey(),
                thread: *thread,
                treasury_shard_idx: 0,
                author_fee_shard_idx: 0,
                candidates: vec![(0, slot)],
                body: text.as_bytes().to_vec(),
                max_fee: u64::MAX,
                reply: None,
                extend: None,
            },
        ),
        &[author],
    );
}

#[test]
fn fill_slot_multi_candidate_picks_first_free() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();

    author_post(&mut env, &author, &thread_key, 0, "first");

    // Candidates [(0,0) occupied, (0,1) free] → the program fills slot 1.
    env.send_ok(
        ix_fill_slot_ex(
            &env.program_id,
            FillArgs {
                payer: author.pubkey(),
                thread: thread_key,
                treasury_shard_idx: 0,
                author_fee_shard_idx: 0,
                candidates: vec![(0, 0), (0, 1)],
                body: b"second".to_vec(),
                max_fee: u64::MAX,
                reply: None,
                extend: None,
            },
        ),
        &[&author],
    );

    let slot0: ContentNode = env.decode(&content_pda(&env.program_id, &thread_key, 0, 0));
    let slot1: ContentNode = env.decode(&content_pda(&env.program_id, &thread_key, 0, 1));
    assert_eq!(slot0.body, b"first".to_vec(), "occupied slot untouched");
    assert_eq!(slot1.body, b"second".to_vec(), "first free slot filled");
}

#[test]
fn fill_slot_rejects_when_all_candidates_occupied() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();

    author_post(&mut env, &author, &thread_key, 0, "taken");

    let res = env.send(
        ix_fill_slot_ex(
            &env.program_id,
            FillArgs {
                payer: author.pubkey(),
                thread: thread_key,
                treasury_shard_idx: 0,
                author_fee_shard_idx: 0,
                candidates: vec![(0, 0)],
                body: b"nope".to_vec(),
                max_fee: u64::MAX,
                reply: None,
                extend: None,
            },
        ),
        &[&author],
    );
    assert_custom_error(res, protocol_code(ProtocolError::NoFreeSlot));
}

#[test]
fn reply_pointer_is_stored_and_survives_parent_deletion() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let bob = env.wallet(100 * LAMPORTS_PER_SOL);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();

    // Parent at (0,0) by author.
    author_post(&mut env, &author, &thread_key, 0, "parent");

    // Reply at (0,1) by bob pointing to (0,0).
    env.send_ok(
        ix_fill_slot_ex(
            &env.program_id,
            FillArgs {
                payer: bob.pubkey(),
                thread: thread_key,
                treasury_shard_idx: 0,
                author_fee_shard_idx: 0,
                candidates: vec![(0, 1)],
                body: b"reply".to_vec(),
                max_fee: u64::MAX,
                reply: Some((0, 0)),
                extend: None,
            },
        ),
        &[&bob],
    );

    let reply_pda = content_pda(&env.program_id, &thread_key, 0, 1);
    let reply: ContentNode = env.decode(&reply_pda);
    assert_eq!(reply.header.reply_alloc_seq, 0);
    assert_eq!(reply.header.reply_slot, 0);

    // Author deletes the parent; the reply keeps its now-dangling pointer.
    let parent_pda = content_pda(&env.program_id, &thread_key, 0, 0);
    env.send_ok(
        ix_close_account(&env.program_id, &author.pubkey(), &parent_pda, None),
        &[&author],
    );

    let still: ContentNode = env.decode(&reply_pda);
    assert_eq!(still.header.author.to_bytes(), bob.pubkey().to_bytes());
    assert_eq!(still.header.reply_alloc_seq, 0);
    assert_eq!(still.header.reply_slot, 0);
}

#[test]
fn multi_chunk_append_reassembles_body() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();

    author_post(&mut env, &author, &thread_key, 0, "AAAA");
    env.send_ok(
        ix_append_content(&env.program_id, &author.pubkey(), &thread_key, 0, 0, 0, 0, b"BBBB".to_vec()),
        &[&author],
    );
    env.send_ok(
        ix_append_content(&env.program_id, &author.pubkey(), &thread_key, 0, 0, 0, 0, b"CCCC".to_vec()),
        &[&author],
    );

    let node: ContentNode = env.decode(&content_pda(&env.program_id, &thread_key, 0, 0));
    assert_eq!(node.body, b"AAAABBBBCCCC".to_vec(), "appended chunks reassemble in order");
}

#[test]
fn entry_fee_gates_posting_for_unpaid_wallet() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();

    env.send_ok(
        ix_init_thread_access(&env.program_id, &author.pubkey(), &thread_key, true, 0),
        &[&author],
    );
    env.send_ok(
        ix_set_entry_fee(&env.program_id, &author.pubkey(), &thread_key, 5_000_000),
        &[&author],
    );

    // Entry fee (with gating enabled) gates the thread even with an empty
    // whitelist: a wallet that hasn't paid cannot post.
    let outsider = env.wallet(100 * LAMPORTS_PER_SOL);
    let res = env.send(
        ix_fill_slot(
            &env.program_id,
            &outsider.pubkey(),
            &thread_key,
            0,
            0,
            0,
            0,
            b"unpaid".to_vec(),
            u64::MAX,
            None,
        ),
        &[&outsider],
    );
    assert_custom_error(res, protocol_code(ProtocolError::AccessDenied));

    // After paying, the same wallet can post.
    env.send_ok(
        ix_request_access(&env.program_id, &outsider.pubkey(), &thread_key, 0, 0),
        &[&outsider],
    );
    env.send_ok(
        ix_fill_slot(
            &env.program_id,
            &outsider.pubkey(),
            &thread_key,
            0,
            0,
            0,
            0,
            b"paid in".to_vec(),
            u64::MAX,
            None,
        ),
        &[&outsider],
    );
}

#[test]
fn remove_from_fee_whitelist_refuses_plain_allow_entry() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();
    env.send_ok(
        ix_init_thread_access(&env.program_id, &author.pubkey(), &thread_key, true, 0),
        &[&author],
    );

    let member = env.wallet(0);
    env.send_ok(
        ix_add_to_whitelist(&env.program_id, &author.pubkey(), &thread_key, &member.pubkey()),
        &[&author],
    );

    // The fee-removal path only closes ACCESS_FEE_EXEMPT entries; an ALLOWED
    // entry must not be closable through it (status mismatch).
    let res = env.send(
        ix_remove_from_fee_whitelist(&env.program_id, &author.pubkey(), &thread_key, &member.pubkey()),
        &[&author],
    );
    assert_custom_error(res, protocol_code(ProtocolError::AccessListMissing));

    // The whitelist entry survived the failed removal.
    let entry_key = access_entry_pda(&env.program_id, &thread_key, &member.pubkey());
    assert!(env.account(&entry_key).map(|a| a.lamports > 0).unwrap_or(false));
}

#[test]
fn emptying_whitelist_reopens_the_thread() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();
    env.send_ok(
        ix_init_thread_access(&env.program_id, &author.pubkey(), &thread_key, true, 0),
        &[&author],
    );

    let member = env.wallet(100 * LAMPORTS_PER_SOL);
    let outsider = env.wallet(100 * LAMPORTS_PER_SOL);
    env.send_ok(
        ix_add_to_whitelist(&env.program_id, &author.pubkey(), &thread_key, &member.pubkey()),
        &[&author],
    );

    // Non-empty whitelist gates the outsider.
    let res = env.send(
        ix_fill_slot(&env.program_id, &outsider.pubkey(), &thread_key, 0, 0, 0, 0, b"blocked".to_vec(), u64::MAX, None),
        &[&outsider],
    );
    assert_custom_error(res, protocol_code(ProtocolError::AccessDenied));

    // Remove the only member → empty whitelist + no entry fee → open again.
    env.send_ok(
        ix_remove_from_whitelist(&env.program_id, &author.pubkey(), &thread_key, &member.pubkey()),
        &[&author],
    );
    env.send_ok(
        ix_fill_slot(&env.program_id, &outsider.pubkey(), &thread_key, 0, 0, 0, 0, b"open again".to_vec(), u64::MAX, None),
        &[&outsider],
    );
    assert!(env.balance(&content_pda(&env.program_id, &thread_key, 0, 0)) > 0);
}

#[test]
fn follower_count_aggregates_across_users() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();

    assert_eq!(env.follower_count_total(&thread_key), 0);

    let alice = env.wallet(100 * LAMPORTS_PER_SOL);
    let bob = env.wallet(100 * LAMPORTS_PER_SOL);
    env.send_ok(ix_subscribe(&env.program_id, &alice.pubkey(), &thread_key), &[&alice]);
    env.send_ok(ix_subscribe(&env.program_id, &bob.pubkey(), &thread_key), &[&bob]);

    // Regardless of shard placement, the aggregate is exactly 2.
    assert_eq!(env.follower_count_total(&thread_key), 2);

    env.send_ok(ix_unsubscribe(&env.program_id, &alice.pubkey(), &thread_key), &[&alice]);
    assert_eq!(env.follower_count_total(&thread_key), 1);
}

#[test]
fn fee_cut_setters_enforce_max_bound() {
    let mut env = Env::new();
    let treasury = env.wallet(0);
    let pd = env.program_data_pda();
    let admin = env.admin.insecure_clone();
    env.send_ok(
        ix_init_settings(&env.program_id, &admin.pubkey(), &pd, &treasury.pubkey()),
        &[&admin],
    );

    // MAX_FEE_CUT_BPS = 5000 applies to every cut setter.
    let res = env.send(ix_set_author_fee_cut(&env.program_id, &admin.pubkey(), 5001), &[&admin]);
    assert_custom_error(res, protocol_code(ProtocolError::InvalidFeeBps));
    let res = env.send(ix_set_entry_cut(&env.program_id, &admin.pubkey(), 5001), &[&admin]);
    assert_custom_error(res, protocol_code(ProtocolError::InvalidFeeBps));
    let res = env.send(ix_set_like_cut(&env.program_id, &admin.pubkey(), 5001), &[&admin]);
    assert_custom_error(res, protocol_code(ProtocolError::InvalidFeeBps));

    // The maximum itself is accepted.
    env.send_ok(ix_set_author_fee_cut(&env.program_id, &admin.pubkey(), 5000), &[&admin]);
}
