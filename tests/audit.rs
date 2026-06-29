//! Security regression tests for findings from the manual audit.

mod common;

use common::*;
use solana_account::Account;
use solana_signer::Signer;
use txtcel_program::error::ProtocolError;

/// L-1: `assert_upgrade_authority` must reject a ProgramData account that is not
/// owned by the BPF upgradeable loader before trusting its bytes. We forge a
/// program-owned account at the canonical ProgramData address with otherwise
/// valid contents (variant 3 + slot + `Some(admin)`); `init_settings` must fail
/// on the owner check rather than accepting the forged upgrade authority.
#[test]
fn init_settings_rejects_programdata_owned_by_non_loader() {
    let mut env = Env::new();
    let treasury = env.wallet(0);
    let admin = env.admin.insecure_clone();

    let programdata = env.program_data_pda();
    let mut data = vec![0u8; 45];
    data[0] = 3; // UpgradeableLoaderState::ProgramData
    data[12] = 1; // Some(authority)
    data[13..45].copy_from_slice(&admin.pubkey().to_bytes());
    let lamports = env.rent(data.len());

    // Same valid layout as a real ProgramData account, but owned by the program
    // itself instead of the upgradeable loader.
    env.svm
        .set_account(
            programdata,
            Account {
                lamports,
                data,
                owner: env.program_id,
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();

    let res = env.send(
        ix_init_settings(&env.program_id, &admin.pubkey(), &programdata, &treasury.pubkey()),
        &[&admin],
    );
    assert_custom_error(res, protocol_code(ProtocolError::AccountOwnerMismatch));
}
