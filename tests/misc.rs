//! Remaining instruction coverage: per-thread fee setters, access toggling,
//! and fee-whitelist removal.

mod common;

use common::*;
use solana_keypair::Keypair;
use solana_signer::Signer;
use txtcel_program::error::ProtocolError;
use txtcel_program::state::{AccessEntry, ThreadAccess, ThreadNode, ACCESS_FEE_EXEMPT};

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
fn set_message_fee_updates_thread_and_is_author_only() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author);
    let thread_key = thread.pubkey();

    env.send_ok(
        ix_set_message_fee(&env.program_id, &author.pubkey(), &thread_key, 9_000_000),
        &[&author],
    );
    let t: ThreadNode = env.decode(&thread_key);
    assert_eq!(t.message_fee, 9_000_000);

    let imposter = env.wallet(10 * LAMPORTS_PER_SOL);
    let res = env.send(
        ix_set_message_fee(&env.program_id, &imposter.pubkey(), &thread_key, 1),
        &[&imposter],
    );
    assert_custom_error(res, protocol_code(ProtocolError::Unauthorized));
}

#[test]
fn set_thread_access_toggles_gating() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author);
    let thread_key = thread.pubkey();

    env.send_ok(
        ix_init_thread_access(&env.program_id, &author.pubkey(), &thread_key, true, 0),
        &[&author],
    );
    let member = env.wallet(100 * LAMPORTS_PER_SOL);
    env.send_ok(
        ix_add_to_whitelist(&env.program_id, &author.pubkey(), &thread_key, &member.pubkey()),
        &[&author],
    );

    // Gating on: outsider denied.
    let outsider = env.wallet(100 * LAMPORTS_PER_SOL);
    let res = env.send(
        ix_fill_slot(&env.program_id, &outsider.pubkey(), &thread_key, 0, 0, 0, 0, b"x".to_vec(), u64::MAX, None),
        &[&outsider],
    );
    assert_custom_error(res, protocol_code(ProtocolError::AccessDenied));

    // Disable gating: outsider may now post.
    env.send_ok(
        ix_set_thread_access(&env.program_id, &author.pubkey(), &thread_key, false),
        &[&author],
    );
    let access: ThreadAccess = env.decode(&access_pda(&env.program_id, &thread_key));
    assert!(!access.enabled);

    env.send_ok(
        ix_fill_slot(&env.program_id, &outsider.pubkey(), &thread_key, 0, 0, 0, 1, b"now ok".to_vec(), u64::MAX, None),
        &[&outsider],
    );
    assert!(env.balance(&content_pda(&env.program_id, &thread_key, 0, 1)) > 0);
}

#[test]
fn remove_from_fee_whitelist_closes_entry() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author);
    let thread_key = thread.pubkey();
    env.send_ok(
        ix_init_thread_access(&env.program_id, &author.pubkey(), &thread_key, false, 0),
        &[&author],
    );

    let vip = env.wallet(0);
    let entry_key = access_entry_pda(&env.program_id, &thread_key, &vip.pubkey());
    env.send_ok(
        ix_add_to_fee_whitelist(&env.program_id, &author.pubkey(), &thread_key, &vip.pubkey()),
        &[&author],
    );
    let entry: AccessEntry = env.decode(&entry_key);
    assert_eq!(entry.status, ACCESS_FEE_EXEMPT);
    // Fee-exempt entries do not count toward the whitelist gate.
    let access: ThreadAccess = env.decode(&access_pda(&env.program_id, &thread_key));
    assert_eq!(access.whitelist_count, 0);

    env.send_ok(
        ix_remove_from_fee_whitelist(&env.program_id, &author.pubkey(), &thread_key, &vip.pubkey()),
        &[&author],
    );
    let emptied = env
        .account(&entry_key)
        .map(|a| a.lamports == 0 && a.data.is_empty())
        .unwrap_or(true);
    assert!(emptied, "fee-whitelist entry closed");
}
