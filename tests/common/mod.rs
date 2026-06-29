//! Shared litesvm test harness.
//!
//! The program is loaded as a compiled `.so` into an in-process SVM. Instruction
//! data is produced by borsh-serializing this crate's own `ProgramInstruction`,
//! and PDAs are derived from the same seed constants the program uses, so the
//! tests can never silently drift from the on-chain layout.
//!
//! Version note: the program's public types use `solana-program` 3.x (`P3`
//! below), while the client side (keypairs, messages, litesvm) uses the
//! fine-grained `solana-*` 3.x crates (`Pk`). The two `Pubkey` types are bridged
//! by raw bytes at the boundary (`to_p3`), which is version-agnostic.

#![allow(dead_code)]
// litesvm's `FailedTransactionMetadata` is a large external type returned by the
// SVM by value; the harness mirrors that signature, so boxing it here buys nothing.
#![allow(clippy::result_large_err)]

use borsh::{to_vec, BorshDeserialize};
use litesvm::types::{FailedTransactionMetadata, TransactionMetadata};
use litesvm::LiteSVM;
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::Transaction;

use solana_program::pubkey::Pubkey as P3;

use txtcel_program::content::CONTENT_SEED;
use txtcel_program::instruction::ProgramInstruction;
use txtcel_program::state::{
    CandidateSlot, FollowerShard, ACCESS_SEED, ACL_SEED, ALLOC_SEED, AUTHOR_FEE_SEED,
    FOLLOWER_COUNT_SEED, FOLLOWS_SEED, LIKES_SEED, N_FOLLOWER_SHARDS, SETTINGS_SEED,
    TREASURY_SHARD_SEED,
};

/// Client-side pubkey type (litesvm / message / keypair side).
pub type Pk = Pubkey;

/// Default `.so` path produced by `cargo build-sbf`, overridable via env.
fn program_so_path() -> String {
    std::env::var("TXTCEL_SO").unwrap_or_else(|_| "target/deploy/txtcel_program.so".to_string())
}

fn bpf_loader_upgradeable() -> Pk {
    "BPFLoaderUpgradeab1e11111111111111111111111"
        .parse()
        .expect("valid loader id")
}

/// System program id is the all-zero pubkey (`111…111`).
pub const SYSTEM_PROGRAM: Pk = Pk::new_from_array([0u8; 32]);

/// Bridge a client pubkey into the program's `solana-program` pubkey type.
pub fn to_p3(key: &Pk) -> P3 {
    P3::new_from_array(key.to_bytes())
}

// ── environment ──

pub struct Env {
    pub svm: LiteSVM,
    pub program_id: Pk,
    /// Upgrade authority of the program AND the initial program admin.
    pub admin: Keypair,
}

impl Env {
    /// Boots an SVM, loads the program, installs a fake upgradeable
    /// program-data account whose upgrade authority is `admin`, and funds admin.
    pub fn new() -> Self {
        let mut svm = LiteSVM::new();
        let program_id = Pk::new_unique();
        svm.add_program_from_file(program_id, program_so_path())
            .expect("load program .so (run `cargo build-sbf` first)");

        let admin = Keypair::new();
        svm.airdrop(&admin.pubkey(), 1_000 * LAMPORTS_PER_SOL)
            .unwrap();

        // Install a fake BPFLoaderUpgradeable ProgramData account at the
        // canonical PDA so `assert_upgrade_authority` accepts `admin`. Layout:
        // 4-byte variant + 8-byte slot + 1-byte Option flag + 32-byte authority.
        let (programdata, _) =
            Pk::find_program_address(&[program_id.as_ref()], &bpf_loader_upgradeable());
        let mut data = vec![0u8; 45];
        data[0] = 3; // UpgradeableLoaderState::ProgramData
        data[12] = 1; // Some(authority)
        data[13..45].copy_from_slice(&admin.pubkey().to_bytes());
        let lamports = svm.minimum_balance_for_rent_exemption(data.len());
        svm.set_account(
            programdata,
            Account {
                lamports,
                data,
                owner: bpf_loader_upgradeable(),
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();

        Self {
            svm,
            program_id,
            admin,
        }
    }

    pub fn program_data_pda(&self) -> Pk {
        Pk::find_program_address(&[self.program_id.as_ref()], &bpf_loader_upgradeable()).0
    }

    /// Creates and funds a fresh wallet.
    pub fn wallet(&mut self, lamports: u64) -> Keypair {
        let kp = Keypair::new();
        self.svm.airdrop(&kp.pubkey(), lamports).unwrap();
        kp
    }

    pub fn balance(&self, key: &Pk) -> u64 {
        self.svm.get_balance(key).unwrap_or(0)
    }

    pub fn rent(&self, size: usize) -> u64 {
        self.svm.minimum_balance_for_rent_exemption(size)
    }

    /// Advances the clock sysvar by `secs` (for testing time-windowed logic).
    pub fn advance_unix_time(&mut self, secs: i64) {
        let mut clock: solana_clock::Clock = self.svm.get_sysvar();
        clock.unix_timestamp += secs;
        self.svm.set_sysvar(&clock);
    }

    pub fn account(&self, key: &Pk) -> Option<Account> {
        self.svm.get_account(key)
    }

    pub fn data(&self, key: &Pk) -> Vec<u8> {
        self.svm.get_account(key).map(|a| a.data).unwrap_or_default()
    }

    pub fn decode<T: BorshDeserialize>(&self, key: &Pk) -> T {
        let data = self.data(key);
        T::try_from_slice(&data).expect("account decodes")
    }

    /// Sends a single-instruction transaction signed by `signers` (payer first).
    pub fn send(
        &mut self,
        ix: Instruction,
        signers: &[&Keypair],
    ) -> Result<TransactionMetadata, FailedTransactionMetadata> {
        self.send_many(&[ix], signers)
    }

    pub fn send_many(
        &mut self,
        ixs: &[Instruction],
        signers: &[&Keypair],
    ) -> Result<TransactionMetadata, FailedTransactionMetadata> {
        self.svm.expire_blockhash();
        let blockhash = self.svm.latest_blockhash();
        let payer = signers[0].pubkey();
        let msg = Message::new(ixs, Some(&payer));
        let tx = Transaction::new(signers, msg, blockhash);
        self.svm.send_transaction(tx)
    }

    /// Sends and panics with logs on failure.
    pub fn send_ok(&mut self, ix: Instruction, signers: &[&Keypair]) -> TransactionMetadata {
        match self.send(ix, signers) {
            Ok(meta) => meta,
            Err(failed) => {
                eprintln!("--- tx failed: {:?}", failed.err);
                for log in &failed.meta.logs {
                    eprintln!("{log}");
                }
                panic!("expected transaction to succeed");
            }
        }
    }
}

pub const LAMPORTS_PER_SOL: u64 = 1_000_000_000;

// ── PDA derivation (client-side, from program seeds) ──

pub fn settings_pda(program_id: &Pk) -> Pk {
    Pk::find_program_address(&[SETTINGS_SEED], program_id).0
}

pub fn alloc_pda(program_id: &Pk, thread: &Pk, alloc_seq: u32) -> Pk {
    Pk::find_program_address(
        &[ALLOC_SEED, thread.as_ref(), &alloc_seq.to_le_bytes()],
        program_id,
    )
    .0
}

pub fn content_pda(program_id: &Pk, thread: &Pk, alloc_seq: u32, slot: u8) -> Pk {
    Pk::find_program_address(
        &[
            CONTENT_SEED,
            thread.as_ref(),
            &alloc_seq.to_le_bytes(),
            &[slot],
        ],
        program_id,
    )
    .0
}

pub fn access_pda(program_id: &Pk, thread: &Pk) -> Pk {
    Pk::find_program_address(&[ACCESS_SEED, thread.as_ref()], program_id).0
}

pub fn access_entry_pda(program_id: &Pk, thread: &Pk, wallet: &Pk) -> Pk {
    Pk::find_program_address(
        &[ACL_SEED, thread.as_ref(), wallet.as_ref()],
        program_id,
    )
    .0
}

pub fn likes_pda(program_id: &Pk, thread: &Pk, alloc_seq: u32) -> Pk {
    Pk::find_program_address(
        &[LIKES_SEED, thread.as_ref(), &alloc_seq.to_le_bytes()],
        program_id,
    )
    .0
}

pub fn treasury_shard_pda(program_id: &Pk, shard: u16) -> Pk {
    Pk::find_program_address(&[TREASURY_SHARD_SEED, &shard.to_le_bytes()], program_id).0
}

pub fn author_fee_pda(program_id: &Pk, thread: &Pk, shard: u8) -> Pk {
    Pk::find_program_address(
        &[AUTHOR_FEE_SEED, thread.as_ref(), &[shard]],
        program_id,
    )
    .0
}

pub fn follow_registry_pda(program_id: &Pk, owner: &Pk) -> Pk {
    Pk::find_program_address(&[FOLLOWS_SEED, owner.as_ref()], program_id).0
}

pub fn follower_shard_pda(program_id: &Pk, thread: &Pk, shard: u8) -> Pk {
    Pk::find_program_address(
        &[FOLLOWER_COUNT_SEED, thread.as_ref(), &[shard]],
        program_id,
    )
    .0
}

pub fn follower_shard_index(wallet: &Pk) -> u8 {
    wallet.to_bytes()[0] % N_FOLLOWER_SHARDS
}

impl Env {
    /// Sums the follower counter across every shard of a thread (mirrors the
    /// SDK's aggregate `loadFollowerCount`).
    pub fn follower_count_total(&self, thread: &Pk) -> u64 {
        (0..N_FOLLOWER_SHARDS)
            .map(|shard| {
                let key = follower_shard_pda(&self.program_id, thread, shard);
                match self.account(&key) {
                    Some(_) => self.decode::<FollowerShard>(&key).count as u64,
                    None => 0,
                }
            })
            .sum()
    }
}

// ── instruction builders ──

fn data(ix: &ProgramInstruction) -> Vec<u8> {
    to_vec(ix).expect("borsh serialize")
}

pub fn ix_init_settings(program_id: &Pk, authority: &Pk, programdata: &Pk, treasury: &Pk) -> Instruction {
    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*authority, true),
            AccountMeta::new(settings_pda(program_id), false),
            AccountMeta::new_readonly(*programdata, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        ],
        data: data(&ProgramInstruction::InitSettings {
            treasury: to_p3(treasury),
        }),
    }
}

pub fn ix_set_treasury(program_id: &Pk, authority: &Pk, treasury: &Pk) -> Instruction {
    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*authority, true),
            AccountMeta::new(settings_pda(program_id), false),
        ],
        data: data(&ProgramInstruction::SetTreasury {
            treasury: to_p3(treasury),
        }),
    }
}

pub fn ix_set_admin(program_id: &Pk, authority: &Pk, new_admin: &Pk) -> Instruction {
    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*authority, true),
            AccountMeta::new(settings_pda(program_id), false),
        ],
        data: data(&ProgramInstruction::SetAdmin {
            new_admin: to_p3(new_admin),
        }),
    }
}

fn settings_setter(program_id: &Pk, authority: &Pk, ix: ProgramInstruction) -> Instruction {
    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*authority, true),
            AccountMeta::new(settings_pda(program_id), false),
        ],
        data: data(&ix),
    }
}

pub fn ix_set_base_fee(program_id: &Pk, authority: &Pk, fee_bps: u32) -> Instruction {
    settings_setter(program_id, authority, ProgramInstruction::SetBaseFee { fee_bps })
}

pub fn ix_set_author_fee_cut(program_id: &Pk, authority: &Pk, fee_bps: u32) -> Instruction {
    settings_setter(program_id, authority, ProgramInstruction::SetAuthorFeeCut { fee_bps })
}

pub fn ix_set_entry_cut(program_id: &Pk, authority: &Pk, fee_bps: u32) -> Instruction {
    settings_setter(program_id, authority, ProgramInstruction::SetEntryCut { fee_bps })
}

pub fn ix_set_like_cut(program_id: &Pk, authority: &Pk, fee_bps: u32) -> Instruction {
    settings_setter(program_id, authority, ProgramInstruction::SetLikeCut { fee_bps })
}

#[allow(clippy::too_many_arguments)]
pub fn ix_create_root_alloc(
    program_id: &Pk,
    payer: &Pk,
    thread: &Pk,
    treasury_shard_idx: u16,
    message_fee: u64,
    title: Vec<u8>,
) -> Instruction {
    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(*thread, true),
            AccountMeta::new(alloc_pda(program_id, thread, 0), false),
            AccountMeta::new_readonly(settings_pda(program_id), false),
            AccountMeta::new(treasury_shard_pda(program_id, treasury_shard_idx), false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        ],
        data: data(&ProgramInstruction::CreateRootAlloc {
            message_fee,
            treasury_shard_idx,
            title,
        }),
    }
}

pub fn ix_prepare_alloc(
    program_id: &Pk,
    payer: &Pk,
    thread: &Pk,
    current_seq: u32,
) -> Instruction {
    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(alloc_pda(program_id, thread, current_seq), false),
            AccountMeta::new(alloc_pda(program_id, thread, current_seq + 1), false),
            AccountMeta::new(*thread, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        ],
        data: data(&ProgramInstruction::PrepareAlloc {
            alloc_seq: current_seq,
        }),
    }
}

/// Builds a `FillSlot` for a single candidate `(alloc_seq, slot)`.
///
/// `fill_slot` no longer grows the alloc chain — linking is done separately via
/// `ix_prepare_alloc`. The `_extend` argument is kept for call-site
/// compatibility and is intentionally ignored.
#[allow(clippy::too_many_arguments)]
pub fn ix_fill_slot(
    program_id: &Pk,
    payer: &Pk,
    thread: &Pk,
    treasury_shard_idx: u16,
    author_fee_shard_idx: u8,
    alloc_seq: u32,
    slot: u8,
    body: Vec<u8>,
    max_fee: u64,
    _extend: Option<(u32, u32)>,
) -> Instruction {
    let candidates = vec![CandidateSlot { alloc_seq, slot }];
    let accounts = vec![
        AccountMeta::new(*payer, true),
        AccountMeta::new_readonly(*thread, false),
        AccountMeta::new_readonly(settings_pda(program_id), false),
        AccountMeta::new(treasury_shard_pda(program_id, treasury_shard_idx), false),
        AccountMeta::new(author_fee_pda(program_id, thread, author_fee_shard_idx), false),
        AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        // candidate content account(s)
        AccountMeta::new(content_pda(program_id, thread, alloc_seq, slot), false),
        // mandatory access + entry PDAs
        AccountMeta::new_readonly(access_pda(program_id, thread), false),
        AccountMeta::new_readonly(access_entry_pda(program_id, thread, payer), false),
    ];
    Instruction {
        program_id: *program_id,
        accounts,
        data: data(&ProgramInstruction::FillSlot {
            kind: 0,
            body,
            candidates,
            treasury_shard_idx,
            author_fee_shard_idx,
            reply_alloc_seq: 0,
            reply_slot: 0,
            max_fee,
        }),
    }
}

/// Flexible `fill_slot` builder supporting multiple candidate slots and an
/// optional reply pointer.
pub struct FillArgs {
    pub payer: Pk,
    pub thread: Pk,
    pub treasury_shard_idx: u16,
    pub author_fee_shard_idx: u8,
    pub candidates: Vec<(u32, u8)>,
    pub body: Vec<u8>,
    pub max_fee: u64,
    pub reply: Option<(u32, u8)>,
    /// Kept for call-site compatibility; `fill_slot` no longer auto-extends, so
    /// this is ignored. Link pages with `ix_prepare_alloc`.
    pub extend: Option<(u32, u32)>,
}

pub fn ix_fill_slot_ex(program_id: &Pk, args: FillArgs) -> Instruction {
    let candidate_slots: Vec<CandidateSlot> = args
        .candidates
        .iter()
        .map(|(alloc_seq, slot)| CandidateSlot {
            alloc_seq: *alloc_seq,
            slot: *slot,
        })
        .collect();
    let (reply_alloc_seq, reply_slot) = args.reply.unwrap_or((0, 0));

    let mut accounts = vec![
        AccountMeta::new(args.payer, true),
        AccountMeta::new_readonly(args.thread, false),
        AccountMeta::new_readonly(settings_pda(program_id), false),
        AccountMeta::new(treasury_shard_pda(program_id, args.treasury_shard_idx), false),
        AccountMeta::new(author_fee_pda(program_id, &args.thread, args.author_fee_shard_idx), false),
        AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
    ];
    for (alloc_seq, slot) in &args.candidates {
        accounts.push(AccountMeta::new(
            content_pda(program_id, &args.thread, *alloc_seq, *slot),
            false,
        ));
    }
    accounts.push(AccountMeta::new_readonly(access_pda(program_id, &args.thread), false));
    accounts.push(AccountMeta::new_readonly(
        access_entry_pda(program_id, &args.thread, &args.payer),
        false,
    ));

    Instruction {
        program_id: *program_id,
        accounts,
        data: data(&ProgramInstruction::FillSlot {
            kind: 0,
            body: args.body,
            candidates: candidate_slots,
            treasury_shard_idx: args.treasury_shard_idx,
            author_fee_shard_idx: args.author_fee_shard_idx,
            reply_alloc_seq,
            reply_slot,
            max_fee: args.max_fee,
        }),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn ix_append_content(
    program_id: &Pk,
    payer: &Pk,
    thread: &Pk,
    alloc_seq: u32,
    slot: u8,
    treasury_shard_idx: u16,
    author_fee_shard_idx: u8,
    chunk: Vec<u8>,
) -> Instruction {
    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(content_pda(program_id, thread, alloc_seq, slot), false),
            AccountMeta::new(*thread, false),
            AccountMeta::new_readonly(settings_pda(program_id), false),
            AccountMeta::new(treasury_shard_pda(program_id, treasury_shard_idx), false),
            AccountMeta::new(author_fee_pda(program_id, thread, author_fee_shard_idx), false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        ],
        data: data(&ProgramInstruction::AppendContent {
            chunk,
            treasury_shard_idx,
            author_fee_shard_idx,
        }),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn ix_like_content(
    program_id: &Pk,
    payer: &Pk,
    thread: &Pk,
    alloc_seq: u32,
    slot: u8,
    treasury_shard_idx: u16,
    author_fee_shard_idx: u8,
    max_fee: u64,
) -> Instruction {
    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(likes_pda(program_id, thread, alloc_seq), false),
            AccountMeta::new_readonly(content_pda(program_id, thread, alloc_seq, slot), false),
            AccountMeta::new_readonly(*thread, false),
            AccountMeta::new_readonly(settings_pda(program_id), false),
            AccountMeta::new(treasury_shard_pda(program_id, treasury_shard_idx), false),
            AccountMeta::new(author_fee_pda(program_id, thread, author_fee_shard_idx), false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        ],
        data: data(&ProgramInstruction::LikeContent {
            alloc_seq,
            slot,
            treasury_shard_idx,
            author_fee_shard_idx,
            max_fee,
        }),
    }
}

pub fn ix_set_message_fee(program_id: &Pk, author: &Pk, thread: &Pk, fee: u64) -> Instruction {
    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*author, true),
            AccountMeta::new(*thread, false),
        ],
        data: data(&ProgramInstruction::SetMessageFee { fee }),
    }
}

pub fn ix_set_like_fee(program_id: &Pk, author: &Pk, thread: &Pk, fee: u64) -> Instruction {
    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*author, true),
            AccountMeta::new(*thread, false),
        ],
        data: data(&ProgramInstruction::SetLikeFee { fee }),
    }
}

pub fn ix_init_thread_access(
    program_id: &Pk,
    authority: &Pk,
    thread: &Pk,
    enabled: bool,
    treasury_shard_idx: u16,
) -> Instruction {
    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*authority, true),
            AccountMeta::new_readonly(*thread, false),
            AccountMeta::new(access_pda(program_id, thread), false),
            AccountMeta::new(treasury_shard_pda(program_id, treasury_shard_idx), false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        ],
        data: data(&ProgramInstruction::InitThreadAccess {
            enabled,
            treasury_shard_idx,
        }),
    }
}

pub fn ix_set_thread_access(program_id: &Pk, authority: &Pk, thread: &Pk, enabled: bool) -> Instruction {
    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*authority, true),
            AccountMeta::new(access_pda(program_id, thread), false),
        ],
        data: data(&ProgramInstruction::SetThreadAccess { enabled }),
    }
}

pub fn ix_set_entry_fee(program_id: &Pk, authority: &Pk, thread: &Pk, fee: u64) -> Instruction {
    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*authority, true),
            AccountMeta::new(access_pda(program_id, thread), false),
        ],
        data: data(&ProgramInstruction::SetEntryFee { fee }),
    }
}

fn acl_setter(program_id: &Pk, authority: &Pk, thread: &Pk, wallet: &Pk, ix: ProgramInstruction) -> Instruction {
    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*authority, true),
            AccountMeta::new(access_pda(program_id, thread), false),
            AccountMeta::new(access_entry_pda(program_id, thread, wallet), false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        ],
        data: data(&ix),
    }
}

pub fn ix_add_to_whitelist(program_id: &Pk, authority: &Pk, thread: &Pk, wallet: &Pk) -> Instruction {
    acl_setter(program_id, authority, thread, wallet, ProgramInstruction::AddToWhitelist { wallet: to_p3(wallet) })
}

pub fn ix_remove_from_whitelist(program_id: &Pk, authority: &Pk, thread: &Pk, wallet: &Pk) -> Instruction {
    acl_setter(program_id, authority, thread, wallet, ProgramInstruction::RemoveFromWhitelist { wallet: to_p3(wallet) })
}

pub fn ix_add_to_blacklist(program_id: &Pk, authority: &Pk, thread: &Pk, wallet: &Pk) -> Instruction {
    acl_setter(program_id, authority, thread, wallet, ProgramInstruction::AddToBlacklist { wallet: to_p3(wallet) })
}

pub fn ix_remove_from_blacklist(program_id: &Pk, authority: &Pk, thread: &Pk, wallet: &Pk) -> Instruction {
    acl_setter(program_id, authority, thread, wallet, ProgramInstruction::RemoveFromBlacklist { wallet: to_p3(wallet) })
}

pub fn ix_add_to_fee_whitelist(program_id: &Pk, authority: &Pk, thread: &Pk, wallet: &Pk) -> Instruction {
    acl_setter(program_id, authority, thread, wallet, ProgramInstruction::AddToFeeWhitelist { wallet: to_p3(wallet) })
}

pub fn ix_remove_from_fee_whitelist(program_id: &Pk, authority: &Pk, thread: &Pk, wallet: &Pk) -> Instruction {
    acl_setter(program_id, authority, thread, wallet, ProgramInstruction::RemoveFromFeeWhitelist { wallet: to_p3(wallet) })
}

#[allow(clippy::too_many_arguments)]
pub fn ix_request_access(
    program_id: &Pk,
    payer: &Pk,
    thread: &Pk,
    treasury_shard_idx: u16,
    author_fee_shard_idx: u8,
) -> Instruction {
    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(access_pda(program_id, thread), false),
            AccountMeta::new(access_entry_pda(program_id, thread, payer), false),
            AccountMeta::new_readonly(*thread, false),
            AccountMeta::new_readonly(settings_pda(program_id), false),
            AccountMeta::new(treasury_shard_pda(program_id, treasury_shard_idx), false),
            AccountMeta::new(author_fee_pda(program_id, thread, author_fee_shard_idx), false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        ],
        data: data(&ProgramInstruction::RequestAccess {
            treasury_shard_idx,
            author_fee_shard_idx,
        }),
    }
}

pub fn ix_subscribe(program_id: &Pk, user: &Pk, thread: &Pk) -> Instruction {
    let shard = follower_shard_index(user);
    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*user, true),
            AccountMeta::new(follow_registry_pda(program_id, user), false),
            AccountMeta::new(follower_shard_pda(program_id, thread, shard), false),
            AccountMeta::new_readonly(*thread, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        ],
        data: data(&ProgramInstruction::Subscribe),
    }
}

pub fn ix_unsubscribe(program_id: &Pk, user: &Pk, thread: &Pk) -> Instruction {
    let shard = follower_shard_index(user);
    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*user, true),
            AccountMeta::new(follow_registry_pda(program_id, user), false),
            AccountMeta::new(follower_shard_pda(program_id, thread, shard), false),
            AccountMeta::new_readonly(*thread, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        ],
        data: data(&ProgramInstruction::Unsubscribe),
    }
}

/// Treasury sweep: `settings`, `treasury_wallet`, then one shard account per index.
pub fn ix_sweep_treasury(
    program_id: &Pk,
    treasury_wallet: &Pk,
    shard_indices: &[u16],
) -> Instruction {
    let mut accounts = vec![
        AccountMeta::new_readonly(settings_pda(program_id), false),
        AccountMeta::new(*treasury_wallet, false),
    ];
    for idx in shard_indices {
        accounts.push(AccountMeta::new(treasury_shard_pda(program_id, *idx), false));
    }
    Instruction {
        program_id: *program_id,
        accounts,
        data: data(&ProgramInstruction::SweepTreasury {
            shard_indices: shard_indices.to_vec(),
        }),
    }
}

/// Author-fee sweep: `thread`, `author_wallet` (signer), then one shard per index.
pub fn ix_sweep_author_fees(
    program_id: &Pk,
    thread: &Pk,
    author_wallet: &Pk,
    shard_indices: &[u8],
) -> Instruction {
    let mut accounts = vec![
        AccountMeta::new_readonly(*thread, false),
        AccountMeta::new(*author_wallet, true),
    ];
    for idx in shard_indices {
        accounts.push(AccountMeta::new(author_fee_pda(program_id, thread, *idx), false));
    }
    Instruction {
        program_id: *program_id,
        accounts,
        data: data(&ProgramInstruction::SweepAuthorFees {
            shard_indices: shard_indices.to_vec(),
        }),
    }
}

/// Close a content account; optionally pass the likes PDA to reset its slot.
pub fn ix_close_account(program_id: &Pk, payer: &Pk, target: &Pk, likes: Option<&Pk>) -> Instruction {
    let mut accounts = vec![
        AccountMeta::new(*payer, true),
        AccountMeta::new(*target, false),
    ];
    if let Some(likes) = likes {
        accounts.push(AccountMeta::new(*likes, false));
    }
    Instruction {
        program_id: *program_id,
        accounts,
        data: data(&ProgramInstruction::CloseAccount),
    }
}

// ── assertion helpers ──

/// Asserts the transaction failed with `ProgramError::Custom(code)`.
pub fn assert_custom_error(result: Result<TransactionMetadata, FailedTransactionMetadata>, code: u32) {
    match result {
        Ok(_) => panic!("expected Custom({code}) error, got success"),
        Err(failed) => {
            let rendered = format!("{:?}", failed.err);
            assert!(
                rendered.contains(&format!("Custom({code})")),
                "expected Custom({code}), got: {rendered}\nlogs: {:#?}",
                failed.meta.logs
            );
        }
    }
}

pub fn protocol_code(err: txtcel_program::error::ProtocolError) -> u32 {
    err as u32
}
