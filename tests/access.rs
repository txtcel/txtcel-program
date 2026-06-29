//! Thread access control: membership entries, gating in `fill_slot`, fee-exempt
//! members, and paid `request_access` with entry-fee splitting.

mod common;

use common::*;
use solana_keypair::Keypair;
use solana_signer::Signer;
use txtcel_program::error::ProtocolError;
use txtcel_program::state::{AccessEntry, ThreadAccess, ACCESS_ALLOWED, ACCESS_DENIED, ACCESS_FEE_EXEMPT};

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

fn init_access(env: &mut Env, author: &Keypair, thread: &Pk, enabled: bool) {
    env.send_ok(
        ix_init_thread_access(&env.program_id, &author.pubkey(), thread, enabled, 0),
        &[author],
    );
}

#[test]
fn init_thread_access_charges_rent_to_treasury_and_is_author_only() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();

    let shard0 = treasury_shard_pda(&env.program_id, 0);
    let treasury_before = env.balance(&shard0);
    let access_rent = env.rent(ThreadAccess::size());

    // Non-author cannot initialize access.
    let stranger = env.wallet(10 * LAMPORTS_PER_SOL);
    let res = env.send(
        ix_init_thread_access(&env.program_id, &stranger.pubkey(), &thread_key, true, 0),
        &[&stranger],
    );
    assert_custom_error(res, protocol_code(ProtocolError::Unauthorized));

    // Author initializes; access rent is collected into the treasury shard.
    init_access(&mut env, &author, &thread_key, true);

    let access_key = access_pda(&env.program_id, &thread_key);
    assert_eq!(env.balance(&access_key), access_rent, "access account rent-exempt");
    assert_eq!(
        env.balance(&shard0),
        treasury_before + access_rent,
        "treasury shard collected the access rent as a fee"
    );
    let access: ThreadAccess = env.decode(&access_key);
    assert!(access.enabled);
    assert_eq!(access.whitelist_count, 0);
}

#[test]
fn whitelist_add_remove_tracks_count_and_rent() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();
    init_access(&mut env, &author, &thread_key, true);

    let member = env.wallet(0);
    let entry_key = access_entry_pda(&env.program_id, &thread_key, &member.pubkey());
    let entry_rent = env.rent(AccessEntry::size());

    // add
    env.send_ok(
        ix_add_to_whitelist(&env.program_id, &author.pubkey(), &thread_key, &member.pubkey()),
        &[&author],
    );
    assert_eq!(env.balance(&entry_key), entry_rent, "entry rent-exempt");
    let entry: AccessEntry = env.decode(&entry_key);
    assert_eq!(entry.status, ACCESS_ALLOWED);
    let access: ThreadAccess = env.decode(&access_pda(&env.program_id, &thread_key));
    assert_eq!(access.whitelist_count, 1);

    // remove (closes entry, refunds rent to author)
    env.send_ok(
        ix_remove_from_whitelist(&env.program_id, &author.pubkey(), &thread_key, &member.pubkey()),
        &[&author],
    );
    let emptied = env
        .account(&entry_key)
        .map(|a| a.lamports == 0 && a.data.is_empty())
        .unwrap_or(true);
    assert!(emptied, "entry closed");
    let access: ThreadAccess = env.decode(&access_pda(&env.program_id, &thread_key));
    assert_eq!(access.whitelist_count, 0);
}

#[test]
fn gating_blocks_non_members_and_allows_whitelisted() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();
    init_access(&mut env, &author, &thread_key, true);

    let member = env.wallet(100 * LAMPORTS_PER_SOL);
    let outsider = env.wallet(100 * LAMPORTS_PER_SOL);

    env.send_ok(
        ix_add_to_whitelist(&env.program_id, &author.pubkey(), &thread_key, &member.pubkey()),
        &[&author],
    );

    // Outsider is rejected (whitelist non-empty + enabled).
    let res = env.send(
        ix_fill_slot(
            &env.program_id,
            &outsider.pubkey(),
            &thread_key,
            0,
            0,
            0,
            0,
            b"hi".to_vec(),
            u64::MAX,
            None,
        ),
        &[&outsider],
    );
    assert_custom_error(res, protocol_code(ProtocolError::AccessDenied));

    // Member is allowed.
    env.send_ok(
        ix_fill_slot(
            &env.program_id,
            &member.pubkey(),
            &thread_key,
            0,
            0,
            0,
            1,
            b"member post".to_vec(),
            u64::MAX,
            None,
        ),
        &[&member],
    );
    let content = content_pda(&env.program_id, &thread_key, 0, 1);
    assert!(env.balance(&content) > 0, "member content created");
}

#[test]
fn blacklist_blocks_posting() {
    let mut env = Env::new();
    let author = setup(&mut env);
    // Open thread (no whitelist), but blacklist one wallet.
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();
    init_access(&mut env, &author, &thread_key, true);

    let banned = env.wallet(100 * LAMPORTS_PER_SOL);
    env.send_ok(
        ix_add_to_blacklist(&env.program_id, &author.pubkey(), &thread_key, &banned.pubkey()),
        &[&author],
    );
    let entry: AccessEntry = env.decode(&access_entry_pda(&env.program_id, &thread_key, &banned.pubkey()));
    assert_eq!(entry.status, ACCESS_DENIED);

    let res = env.send(
        ix_fill_slot(
            &env.program_id,
            &banned.pubkey(),
            &thread_key,
            0,
            0,
            0,
            0,
            b"nope".to_vec(),
            u64::MAX,
            None,
        ),
        &[&banned],
    );
    assert_custom_error(res, protocol_code(ProtocolError::AccessDenied));

    // Un-banning lets them post again.
    env.send_ok(
        ix_remove_from_blacklist(&env.program_id, &author.pubkey(), &thread_key, &banned.pubkey()),
        &[&author],
    );
    env.send_ok(
        ix_fill_slot(
            &env.program_id,
            &banned.pubkey(),
            &thread_key,
            0,
            0,
            0,
            0,
            b"back".to_vec(),
            u64::MAX,
            None,
        ),
        &[&banned],
    );
}

#[test]
fn fee_exempt_member_pays_no_author_fee() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let message_fee = 3_000_000u64;
    let thread = create_channel(&mut env, &author, message_fee);
    let thread_key = thread.pubkey();
    // gating disabled, so anyone can post; we only test the fee waiver.
    init_access(&mut env, &author, &thread_key, false);

    let vip = env.wallet(100 * LAMPORTS_PER_SOL);
    env.send_ok(
        ix_add_to_fee_whitelist(&env.program_id, &author.pubkey(), &thread_key, &vip.pubkey()),
        &[&author],
    );
    let entry: AccessEntry = env.decode(&access_entry_pda(&env.program_id, &thread_key, &vip.pubkey()));
    assert_eq!(entry.status, ACCESS_FEE_EXEMPT);

    let author_shard0 = author_fee_pda(&env.program_id, &thread_key, 0);
    let shard0 = treasury_shard_pda(&env.program_id, 0);
    let treasury_before = env.balance(&shard0);
    let body = b"vip".to_vec();
    let base_fee = pct(env.rent(txtcel_program::content::ContentNode::size(body.len())), 1000);

    env.send_ok(
        ix_fill_slot(
            &env.program_id,
            &vip.pubkey(),
            &thread_key,
            0,
            0,
            0,
            0,
            body,
            u64::MAX,
            None,
        ),
        &[&vip],
    );

    // Author fee shard freshly created, holds only its rent (no author fee).
    assert_eq!(env.balance(&author_shard0), env.rent(0), "no author fee charged");
    assert_eq!(
        env.balance(&shard0),
        treasury_before + base_fee,
        "only the base fee reached the treasury"
    );
}

#[test]
fn request_access_splits_entry_fee_and_grants_membership() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();
    init_access(&mut env, &author, &thread_key, true);

    let entry_fee = 5_000_000u64;
    env.send_ok(
        ix_set_entry_fee(&env.program_id, &author.pubkey(), &thread_key, entry_fee),
        &[&author],
    );

    let buyer = env.wallet(100 * LAMPORTS_PER_SOL);
    let shard0 = treasury_shard_pda(&env.program_id, 0);
    let author_shard0 = author_fee_pda(&env.program_id, &thread_key, 0);
    let treasury_before = env.balance(&shard0);

    let platform_cut = pct(entry_fee, 1000); // entry_cut_bps default
    let author_receives = entry_fee - platform_cut;

    env.send_ok(
        ix_request_access(&env.program_id, &buyer.pubkey(), &thread_key, 0, 0),
        &[&buyer],
    );

    // Entry fee split between treasury and author shard.
    assert_eq!(
        env.balance(&shard0),
        treasury_before + platform_cut,
        "treasury += platform cut of entry fee"
    );
    assert_eq!(
        env.balance(&author_shard0),
        env.rent(0) + author_receives,
        "author shard += author's share of entry fee"
    );

    // Membership granted and counted.
    let entry: AccessEntry = env.decode(&access_entry_pda(&env.program_id, &thread_key, &buyer.pubkey()));
    assert_eq!(entry.status, ACCESS_ALLOWED);
    let access: ThreadAccess = env.decode(&access_pda(&env.program_id, &thread_key));
    assert_eq!(access.whitelist_count, 1);

    // Duplicate request rejected.
    let res = env.send(
        ix_request_access(&env.program_id, &buyer.pubkey(), &thread_key, 0, 0),
        &[&buyer],
    );
    assert_custom_error(res, protocol_code(ProtocolError::AccessListDuplicate));
}

#[test]
fn request_access_requires_nonzero_entry_fee() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();
    init_access(&mut env, &author, &thread_key, true);

    let buyer = env.wallet(100 * LAMPORTS_PER_SOL);
    let res = env.send(
        ix_request_access(&env.program_id, &buyer.pubkey(), &thread_key, 0, 0),
        &[&buyer],
    );
    assert_custom_error(res, protocol_code(ProtocolError::ZeroEntryFee));
}

#[test]
fn request_access_rejected_for_blacklisted() {
    let mut env = Env::new();
    let author = setup(&mut env);
    let thread = create_channel(&mut env, &author, 0);
    let thread_key = thread.pubkey();
    init_access(&mut env, &author, &thread_key, true);
    env.send_ok(
        ix_set_entry_fee(&env.program_id, &author.pubkey(), &thread_key, 1_000_000),
        &[&author],
    );

    let banned = env.wallet(100 * LAMPORTS_PER_SOL);
    env.send_ok(
        ix_add_to_blacklist(&env.program_id, &author.pubkey(), &thread_key, &banned.pubkey()),
        &[&author],
    );

    let res = env.send(
        ix_request_access(&env.program_id, &banned.pubkey(), &thread_key, 0, 0),
        &[&banned],
    );
    assert_custom_error(res, protocol_code(ProtocolError::AccessDenied));
}
