//! Settings & admin instructions: authority checks, fee bounds, admin transfer,
//! and that fee settings actually drive on-chain fee math.

mod common;

use common::*;
use solana_signer::Signer;
use txtcel_program::error::ProtocolError;
use txtcel_program::state::{AllocNode, ProgramSettings, ThreadNode};

fn pct(amount: u64, bps: u32) -> u64 {
    ((amount as u128) * (bps as u128) / 10_000) as u64
}

fn init_with_admin(env: &mut Env, treasury: &Pk) {
    let pd = env.program_data_pda();
    let admin = env.admin.insecure_clone();
    env.send_ok(
        ix_init_settings(&env.program_id, &admin.pubkey(), &pd, treasury),
        &[&admin],
    );
}

#[test]
fn init_settings_rejects_non_upgrade_authority() {
    let mut env = Env::new();
    let imposter = env.wallet(10 * LAMPORTS_PER_SOL);
    let pd = env.program_data_pda();
    let treasury = env.wallet(0);
    // `imposter` signs, but program-data's upgrade authority is `admin`.
    let res = env.send(
        ix_init_settings(&env.program_id, &imposter.pubkey(), &pd, &treasury.pubkey()),
        &[&imposter],
    );
    assert_custom_error(res, protocol_code(ProtocolError::Unauthorized));
}

#[test]
fn init_settings_rejects_double_init() {
    let mut env = Env::new();
    let treasury = env.wallet(0);
    init_with_admin(&mut env, &treasury.pubkey());

    let pd = env.program_data_pda();
    let admin = env.admin.insecure_clone();
    let res = env.send(
        ix_init_settings(&env.program_id, &admin.pubkey(), &pd, &treasury.pubkey()),
        &[&admin],
    );
    assert_custom_error(res, protocol_code(ProtocolError::AccountAlreadyInitialized));
}

#[test]
fn set_admin_transfers_control() {
    let mut env = Env::new();
    let treasury = env.wallet(0);
    init_with_admin(&mut env, &treasury.pubkey());

    let new_admin = env.wallet(10 * LAMPORTS_PER_SOL);
    let admin = env.admin.insecure_clone();
    env.send_ok(
        ix_set_admin(&env.program_id, &admin.pubkey(), &new_admin.pubkey()),
        &[&admin],
    );

    let settings: ProgramSettings = env.decode(&settings_pda(&env.program_id));
    assert_eq!(settings.admin.to_bytes(), new_admin.pubkey().to_bytes());

    // Old admin can no longer change settings.
    let other = env.wallet(0);
    let res = env.send(
        ix_set_treasury(&env.program_id, &admin.pubkey(), &other.pubkey()),
        &[&admin],
    );
    assert_custom_error(res, protocol_code(ProtocolError::Unauthorized));

    // New admin can.
    env.send_ok(
        ix_set_treasury(&env.program_id, &new_admin.pubkey(), &other.pubkey()),
        &[&new_admin],
    );
    let settings: ProgramSettings = env.decode(&settings_pda(&env.program_id));
    assert_eq!(settings.treasury.to_bytes(), other.pubkey().to_bytes());
}

#[test]
fn set_treasury_rejects_non_admin() {
    let mut env = Env::new();
    let treasury = env.wallet(0);
    init_with_admin(&mut env, &treasury.pubkey());

    let imposter = env.wallet(10 * LAMPORTS_PER_SOL);
    let res = env.send(
        ix_set_treasury(&env.program_id, &imposter.pubkey(), &imposter.pubkey()),
        &[&imposter],
    );
    assert_custom_error(res, protocol_code(ProtocolError::Unauthorized));
}

#[test]
fn set_base_fee_rejects_over_max_and_non_admin() {
    let mut env = Env::new();
    let treasury = env.wallet(0);
    init_with_admin(&mut env, &treasury.pubkey());
    let admin = env.admin.insecure_clone();

    // > MAX_FEE_CUT_BPS (5000)
    let res = env.send(ix_set_base_fee(&env.program_id, &admin.pubkey(), 5001), &[&admin]);
    assert_custom_error(res, protocol_code(ProtocolError::InvalidFeeBps));

    // non-admin
    let imposter = env.wallet(10 * LAMPORTS_PER_SOL);
    let res = env.send(ix_set_base_fee(&env.program_id, &imposter.pubkey(), 100), &[&imposter]);
    assert_custom_error(res, protocol_code(ProtocolError::Unauthorized));
}

#[test]
fn zero_base_fee_collects_nothing() {
    let mut env = Env::new();
    let treasury = env.wallet(0);
    init_with_admin(&mut env, &treasury.pubkey());
    let admin = env.admin.insecure_clone();

    env.send_ok(ix_set_base_fee(&env.program_id, &admin.pubkey(), 0), &[&admin]);

    let author = env.wallet(100 * LAMPORTS_PER_SOL);
    let thread = solana_keypair::Keypair::new();
    env.send_ok(
        ix_create_root_alloc(
            &env.program_id,
            &author.pubkey(),
            &thread.pubkey(),
            0,
            0,
            b"chan".to_vec(),
        ),
        &[&author, &thread],
    );

    // With base_fee_bps = 0, the treasury shard holds only its own rent.
    let shard0 = treasury_shard_pda(&env.program_id, 0);
    assert_eq!(env.balance(&shard0), env.rent(0), "no base fee collected");

    // Sanity: created accounts still exist with correct state.
    let _alloc: AllocNode = env.decode(&alloc_pda(&env.program_id, &thread.pubkey(), 0));
    let thread_state: ThreadNode = env.decode(&thread.pubkey());
    assert_eq!(thread_state.alloc_count, 1);
}

#[test]
fn custom_base_fee_changes_collected_amount() {
    let mut env = Env::new();
    let treasury = env.wallet(0);
    init_with_admin(&mut env, &treasury.pubkey());
    let admin = env.admin.insecure_clone();

    // 25%
    env.send_ok(ix_set_base_fee(&env.program_id, &admin.pubkey(), 2500), &[&admin]);

    let author = env.wallet(100 * LAMPORTS_PER_SOL);
    let thread = solana_keypair::Keypair::new();
    let thread_rent = env.rent(ThreadNode::size(b"chan".len()));
    let alloc_rent = env.rent(AllocNode::size());
    let expected = pct(thread_rent + alloc_rent, 2500);

    env.send_ok(
        ix_create_root_alloc(
            &env.program_id,
            &author.pubkey(),
            &thread.pubkey(),
            0,
            0,
            b"chan".to_vec(),
        ),
        &[&author, &thread],
    );

    let shard0 = treasury_shard_pda(&env.program_id, 0);
    assert_eq!(env.balance(&shard0), env.rent(0) + expected, "25% base fee");
}
