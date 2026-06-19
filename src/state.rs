use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::AccountInfo,
    entrypoint::ProgramResult,
    program::invoke,
    program::invoke_signed,
    program_error::ProgramError,
    pubkey::Pubkey,
    sysvar::{rent::Rent, Sysvar},
};
use solana_system_interface::{instruction as system_instruction, program as system_program};

use crate::error::ProtocolError;

// Content node structure now lives in its own module (`crate::content`) for
// decomposition. Re-exported here so existing `use crate::state::*` consumers
// keep resolving `ContentNode`, `load_content`, `derive_content_pda`, etc.
pub use crate::content::*;

// ── constants ──

pub const TAG_CONTENT: u8 = 1;
pub const TAG_ALLOC: u8 = 2;
pub const TAG_THREAD: u8 = 3;
pub const TAG_SETTINGS: u8 = 5;
pub const TAG_ACCESS: u8 = 6;
pub const TAG_LIKES: u8 = 7;
pub const TAG_ACCESS_ENTRY: u8 = 9;

// access entry status
pub const ACCESS_ALLOWED: u8 = 0;
pub const ACCESS_DENIED: u8 = 1;
/// Member who is both allowed to post in a gated thread AND exempt from the
/// per-message author fee. Supersedes `ACCESS_ALLOWED`: a fee-exempt wallet is
/// always allowed, it just additionally pays no `ThreadNode.message_fee`.
pub const ACCESS_FEE_EXEMPT: u8 = 2;

pub const CHILDREN_LEN: usize = 32;
pub const NEXT_ALLOC_INDEX: usize = 31;
pub const EXTEND_THRESHOLD: usize = 16;
pub const MAX_TITLE_LEN: usize = 64;
pub const INDEX_NONE: u32 = u32::MAX;
pub const N_TREASURY_SHARDS: u16 = 512;
pub const N_AUTHOR_FEE_SHARDS: u8 = 4;
pub const MAX_FEE_CUT_BPS: u32 = 5_000;

pub const SETTINGS_SEED: &[u8] = b"settings";
pub const ALLOC_SEED: &[u8] = b"alloc";
pub const ACCESS_SEED: &[u8] = b"access";
pub const ACL_SEED: &[u8] = b"acl";
pub const LIKES_SEED: &[u8] = b"likes";
pub const TREASURY_SHARD_SEED: &[u8] = b"treasury_shard";
pub const AUTHOR_FEE_SEED: &[u8] = b"author_fee";

// ── account structures ──

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct AllocNode {
    pub tag: u8,
    pub thread: Pubkey,
    pub alloc_seq: u32,
    pub upper_alloc_seq: u32,
    pub next_alloc_seq: u32,
}

impl AllocNode {
    pub fn size() -> usize {
        1 + 32 + 4 + 4 + 4
    }
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct ThreadNode {
    pub tag: u8,
    pub alloc_count: u32,
    pub last_alloc_seq: u32,
    pub author: Pubkey,
    /// Fixed per-message fee in lamports a non-author pays to post in this
    /// thread. Set by the thread author. Split author/platform via
    /// `ProgramSettings.author_fee_cut_bps`.
    pub message_fee: u64,
    pub like_fee: u64,
    pub title: Vec<u8>,
}

impl ThreadNode {
    pub fn size(title_len: usize) -> usize {
        1 + 4 + 4 + 32 + 8 + 8 + 4 + title_len
    }
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct ProgramSettings {
    pub tag: u8,
    pub admin: Pubkey,
    pub treasury: Pubkey,
    pub base_fee_bps: u32,
    pub author_fee_cut_bps: u32,
    pub entry_cut_bps: u32,
    pub like_cut_bps: u32,
}

impl ProgramSettings {
    pub fn size() -> usize {
        1 + 32 + 32 + 4 * 4
    }
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct ThreadAccess {
    pub tag: u8,
    pub thread: Pubkey,
    pub enabled: bool,
    pub admin: Pubkey,
    pub entry_fee: u64,
    /// Number of live `ACCESS_ALLOWED` membership entries (the whitelist). Lets
    /// the program know whether the whitelist is empty without scanning every
    /// `AccessEntry` PDA. When `enabled` is set, posting is restricted to
    /// members only if this is non-zero or an `entry_fee` is charged; an empty
    /// whitelist with no entry fee leaves the thread open to everyone except
    /// blacklisted wallets.
    pub whitelist_count: u32,
}

impl ThreadAccess {
    pub fn size() -> usize {
        1 + 32 + 1 + 32 + 8 + 4
    }
}

/// Per-wallet membership record for thread access control.
/// PDA: [ACL_SEED, thread, wallet]. Status is ACCESS_ALLOWED or ACCESS_DENIED.
/// Existence + status give O(1) whitelist/blacklist checks with no size limit.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct AccessEntry {
    pub tag: u8,
    pub thread: Pubkey,
    pub wallet: Pubkey,
    pub status: u8,
}

impl AccessEntry {
    pub fn size() -> usize {
        1 + 32 + 32 + 1
    }
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct AllocLikes {
    pub tag: u8,
    pub alloc_seq: u32,
    pub counts: [u32; NEXT_ALLOC_INDEX],
}

impl AllocLikes {
    pub fn size() -> usize {
        1 + 4 + 4 * NEXT_ALLOC_INDEX
    }
}

// ── candidate slot (instruction data) ──

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct CandidateSlot {
    pub alloc_seq: u32,
    pub slot: u8,
}

// ── validation helpers ──

pub fn assert_signer(account: &AccountInfo) -> ProgramResult {
    if account.is_signer {
        Ok(())
    } else {
        Err(ProtocolError::MissingSigner.into())
    }
}

pub fn assert_writable(account: &AccountInfo) -> ProgramResult {
    if account.is_writable {
        Ok(())
    } else {
        Err(ProtocolError::NotWritable.into())
    }
}

pub fn assert_uninitialized(account: &AccountInfo) -> ProgramResult {
    if system_program::check_id(account.owner) && account.data_len() == 0 {
        Ok(())
    } else {
        Err(ProtocolError::AccountAlreadyInitialized.into())
    }
}

pub fn is_uninitialized(account: &AccountInfo) -> bool {
    system_program::check_id(account.owner) && account.data_len() == 0
}

pub fn ensure_rent_exempt<'a>(
    payer: &AccountInfo<'a>,
    target: &AccountInfo<'a>,
    system_program_account: &AccountInfo<'a>,
    data_len: usize,
) -> ProgramResult {
    let rent = Rent::get()?;
    let minimum = rent.minimum_balance(data_len);
    let current = target.lamports();
    if current >= minimum {
        return Ok(());
    }
    let top_up = minimum
        .checked_sub(current)
        .ok_or(ProtocolError::InvalidAccountData)?;
    if top_up > 0 {
        invoke(
            &system_instruction::transfer(payer.key, target.key, top_up),
            &[
                payer.clone(),
                target.clone(),
                system_program_account.clone(),
            ],
        )?;
    }
    Ok(())
}

pub fn compute_fee_split(amount: u64, cut_bps: u32) -> (u64, u64) {
    let platform = (amount as u128)
        .saturating_mul(cut_bps as u128)
        / 10_000;
    let platform = platform as u64;
    (amount.saturating_sub(platform), platform)
}

pub fn load_thread_access(account: &AccountInfo) -> Result<ThreadAccess, ProgramError> {
    let mut data = &account.data.borrow()[..];
    let access = ThreadAccess::deserialize(&mut data)
        .map_err(|_| ProtocolError::InvalidAccountData)?;
    if access.tag != TAG_ACCESS {
        return Err(ProtocolError::InvalidTag.into());
    }
    Ok(access)
}

pub fn load_access_entry(account: &AccountInfo) -> Result<AccessEntry, ProgramError> {
    let mut data = &account.data.borrow()[..];
    let entry = AccessEntry::deserialize(&mut data)
        .map_err(|_| ProtocolError::InvalidAccountData)?;
    if entry.tag != TAG_ACCESS_ENTRY {
        return Err(ProtocolError::InvalidTag.into());
    }
    Ok(entry)
}

/// Creates the `AccessEntry` PDA for `wallet` if missing, otherwise flips its
/// status. `payer` funds creation. Used by whitelist/blacklist management.
///
/// Returns the entry's previous status: `None` when the entry was newly
/// created, or `Some(prev)` with the status it held before this call. Callers
/// use this to keep the thread's whitelist counter accurate across status
/// transitions.
pub fn set_access_entry_status<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    entry_account: &AccountInfo<'a>,
    system_program_account: &AccountInfo<'a>,
    thread: &Pubkey,
    wallet: &Pubkey,
    status: u8,
) -> Result<Option<u8>, ProgramError> {
    let (expected_entry, entry_bump) = derive_access_entry_pda(program_id, thread, wallet);
    if *entry_account.key != expected_entry {
        return Err(ProtocolError::InvalidPda.into());
    }

    if is_uninitialized(entry_account) {
        let size = AccessEntry::size();

        create_pda_account(
            program_id,
            payer,
            entry_account,
            system_program_account,
            size,
            &[ACL_SEED, thread.as_ref(), wallet.as_ref(), &[entry_bump]],
        )?;

        let entry = AccessEntry {
            tag: TAG_ACCESS_ENTRY,
            thread: *thread,
            wallet: *wallet,
            status,
        };
        entry.serialize(&mut &mut entry_account.data.borrow_mut()[..])?;

        Ok(None)
    } else {
        assert_owned_by(entry_account, program_id)?;
        let mut entry = load_access_entry(entry_account)?;
        if entry.thread != *thread || entry.wallet != *wallet {
            return Err(ProtocolError::ThreadMismatch.into());
        }
        let prev = entry.status;
        entry.status = status;
        entry.serialize(&mut &mut entry_account.data.borrow_mut()[..])?;

        Ok(Some(prev))
    }
}

/// Applies a signed delta to the thread's whitelist member counter, saturating
/// at zero, and persists it. `access_account` must be the program-owned,
/// writable `ThreadAccess` PDA (callers obtain it via `load_admin_access` or an
/// equivalent ownership check).
pub fn adjust_whitelist_count(access_account: &AccountInfo, delta: i64) -> ProgramResult {
    let mut access = load_thread_access(access_account)?;
    access.whitelist_count = (access.whitelist_count as i64)
        .saturating_add(delta)
        .max(0) as u32;
    access.serialize(&mut &mut access_account.data.borrow_mut()[..])?;
    Ok(())
}

/// Closes an `AccessEntry`, returning its rent to `recipient`. Requires the
/// entry to currently hold `expected_status` (so "remove from whitelist" only
/// removes allow-entries and vice-versa).
pub fn close_access_entry<'a>(
    program_id: &Pubkey,
    recipient: &AccountInfo<'a>,
    entry_account: &AccountInfo<'a>,
    _system_program_account: &AccountInfo<'a>,
    thread: &Pubkey,
    wallet: &Pubkey,
    expected_status: u8,
) -> ProgramResult {
    let (expected_entry, _) = derive_access_entry_pda(program_id, thread, wallet);
    if *entry_account.key != expected_entry {
        return Err(ProtocolError::InvalidPda.into());
    }

    if is_uninitialized(entry_account) {
        return Err(ProtocolError::AccessListMissing.into());
    }

    assert_owned_by(entry_account, program_id)?;
    let entry = load_access_entry(entry_account)?;
    if entry.thread != *thread || entry.wallet != *wallet {
        return Err(ProtocolError::ThreadMismatch.into());
    }
    if entry.status != expected_status {
        return Err(ProtocolError::AccessListMissing.into());
    }

    let lamports = entry_account.lamports();
    **recipient.lamports.borrow_mut() = recipient
        .lamports()
        .checked_add(lamports)
        .ok_or(ProtocolError::InvalidAccountData)?;
    **entry_account.lamports.borrow_mut() = 0;
    entry_account.data.borrow_mut().fill(0);
    entry_account.resize(0)?;
    entry_account.assign(&system_program::ID);

    Ok(())
}

pub fn read_tag(account: &AccountInfo) -> Result<u8, ProgramError> {
    let data = account.data.borrow();
    if data.is_empty() {
        return Err(ProtocolError::InvalidAccountData.into());
    }
    Ok(data[0])
}

const BPF_LOADER_UPGRADEABLE_ID: Pubkey =
    solana_program::pubkey!("BPFLoaderUpgradeab1e11111111111111111111111");

pub fn assert_upgrade_authority(
    program_id: &Pubkey,
    programdata_account: &AccountInfo,
    authority: &AccountInfo,
) -> ProgramResult {
    let (expected_programdata, _) = Pubkey::find_program_address(
        &[program_id.as_ref()],
        &BPF_LOADER_UPGRADEABLE_ID,
    );
    if *programdata_account.key != expected_programdata {
        return Err(ProtocolError::InvalidPda.into());
    }

    let data = programdata_account.data.borrow();
    if data.len() < 45 || data[12] != 1 {
        return Err(ProtocolError::Unauthorized.into());
    }
    let upgrade_authority = Pubkey::from(
        <[u8; 32]>::try_from(&data[13..45])
            .map_err(|_| ProtocolError::InvalidAccountData)?,
    );
    if *authority.key != upgrade_authority {
        return Err(ProtocolError::Unauthorized.into());
    }
    Ok(())
}

// ── PDA derivation ──

pub fn derive_settings_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[SETTINGS_SEED], program_id)
}

pub fn derive_alloc_pda(program_id: &Pubkey, thread: &Pubkey, alloc_seq: u32) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[ALLOC_SEED, thread.as_ref(), &alloc_seq.to_le_bytes()], program_id)
}

pub fn derive_access_pda(program_id: &Pubkey, thread: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[ACCESS_SEED, thread.as_ref()], program_id)
}

pub fn derive_access_entry_pda(
    program_id: &Pubkey,
    thread: &Pubkey,
    wallet: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[ACL_SEED, thread.as_ref(), wallet.as_ref()], program_id)
}

pub fn derive_likes_pda(program_id: &Pubkey, thread: &Pubkey, alloc_seq: u32) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[LIKES_SEED, thread.as_ref(), &alloc_seq.to_le_bytes()], program_id)
}

pub fn derive_treasury_shard_pda(program_id: &Pubkey, shard: u16) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[TREASURY_SHARD_SEED, &shard.to_le_bytes()], program_id)
}

pub fn derive_author_fee_pda(program_id: &Pubkey, thread: &Pubkey, shard: u8) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[AUTHOR_FEE_SEED, thread.as_ref(), &[shard]], program_id)
}

// ── system helpers ──

pub fn assert_system_program(account: &AccountInfo) -> ProgramResult {
    if system_program::check_id(account.key) {
        Ok(())
    } else {
        Err(ProgramError::IncorrectProgramId)
    }
}

pub fn assert_owned_by(account: &AccountInfo, owner: &Pubkey) -> ProgramResult {
    if account.owner == owner {
        Ok(())
    } else {
        Err(ProtocolError::AccountOwnerMismatch.into())
    }
}

// ── loaders ──

/// Loads a `ThreadNode`. The thread is a plain program-owned account whose
/// address is its own identity (a full pubkey, not a PDA), so validation is
/// limited to ownership + tag. Child PDAs are bound to this account's key.
pub fn load_thread(program_id: &Pubkey, account: &AccountInfo) -> Result<ThreadNode, ProgramError> {
    assert_owned_by(account, program_id)?;

    let thread = ThreadNode::try_from_slice(&account.data.borrow())
        .map_err(|_| ProtocolError::InvalidAccountData)?;

    if thread.tag != TAG_THREAD {
        return Err(ProtocolError::InvalidTag.into());
    }

    Ok(thread)
}

pub fn load_alloc(program_id: &Pubkey, account: &AccountInfo) -> Result<AllocNode, ProgramError> {
    assert_owned_by(account, program_id)?;

    let alloc = AllocNode::try_from_slice(&account.data.borrow())
        .map_err(|_| ProtocolError::InvalidAccountData)?;

    if alloc.tag != TAG_ALLOC {
        return Err(ProtocolError::InvalidTag.into());
    }

    let (expected, _) = derive_alloc_pda(program_id, &alloc.thread, alloc.alloc_seq);

    if *account.key != expected {
        return Err(ProtocolError::InvalidPda.into());
    }

    Ok(alloc)
}

pub fn load_admin_access(
    program_id: &Pubkey,
    authority: &AccountInfo,
    access_account: &AccountInfo,
) -> Result<ThreadAccess, ProgramError> {
    assert_signer(authority)?;
    assert_writable(access_account)?;
    assert_owned_by(access_account, program_id)?;

    let access = load_thread_access(access_account)?;

    if access.admin != *authority.key {
        return Err(ProtocolError::Unauthorized.into());
    }

    let (expected, _) = derive_access_pda(program_id, &access.thread);

    if *access_account.key != expected {
        return Err(ProtocolError::InvalidPda.into());
    }

    Ok(access)
}

pub fn load_settings(
    program_id: &Pubkey,
    settings_account: &AccountInfo,
) -> Result<ProgramSettings, ProgramError> {
    let (expected_settings, _) = derive_settings_pda(program_id);

    if *settings_account.key != expected_settings {
        return Err(ProtocolError::InvalidPda.into());
    }

    if settings_account.owner != program_id {
        return Err(ProtocolError::AccountOwnerMismatch.into());
    }

    let settings = ProgramSettings::try_from_slice(&settings_account.data.borrow())
        .map_err(|_| ProtocolError::InvalidAccountData)?;

    if settings.tag != TAG_SETTINGS {
        return Err(ProtocolError::InvalidTag.into());
    }

    Ok(settings)
}

// ── fee helpers ──

pub fn collect_fee_to_shard<'a>(
    amount: u64,
    payer: &AccountInfo<'a>,
    shard: &AccountInfo<'a>,
    system_program_account: &AccountInfo<'a>,
) -> ProgramResult {
    if amount == 0 {
        return Ok(());
    }

    let rent = Rent::get()?;
    let shard_rent_min = rent.minimum_balance(0);
    let fee = std::cmp::max(amount, shard_rent_min.saturating_sub(shard.lamports()));

    if fee > 0 {
        invoke(
            &system_instruction::transfer(payer.key, shard.key, fee),
            &[payer.clone(), shard.clone(), system_program_account.clone()],
        )?;
    }
    Ok(())
}

pub fn collect_base_fee<'a>(
    rent_lamports: u64,
    base_fee_bps: u32,
    payer: &AccountInfo<'a>,
    treasury_shard: &AccountInfo<'a>,
    system_program_account: &AccountInfo<'a>,
) -> ProgramResult {

    let base_fee = (rent_lamports as u128).saturating_mul(base_fee_bps as u128) / 10_000;

    collect_fee_to_shard(base_fee as u64, payer, treasury_shard, system_program_account)
}

pub fn transfer_fee_split<'a>(
    amount: u64,
    cut_bps: u32,
    payer: &AccountInfo<'a>,
    author_fee_shard: &AccountInfo<'a>,
    treasury_shard: &AccountInfo<'a>,
    system_program_account: &AccountInfo<'a>,
) -> ProgramResult {
    let (author_receives, platform_cut) = compute_fee_split(amount, cut_bps);

    if platform_cut > 0 {
        collect_fee_to_shard(platform_cut, payer, treasury_shard, system_program_account)?;
    }

    if author_receives > 0 {
        collect_fee_to_shard(author_receives, payer, author_fee_shard, system_program_account)?;
    }

    Ok(())
}

// ── treasury shard validation ──

pub fn validate_treasury_shard(
    program_id: &Pubkey,
    shard_account: &AccountInfo,
    shard_idx: u16,
) -> Result<u8, ProgramError> {
    if shard_idx >= N_TREASURY_SHARDS {
        return Err(ProtocolError::InvalidShard.into());
    }

    let (expected, bump) = derive_treasury_shard_pda(program_id, shard_idx);

    if *shard_account.key != expected {
        return Err(ProtocolError::InvalidPda.into());
    }

    Ok(bump)
}

pub fn validate_author_fee_shard(
    program_id: &Pubkey,
    thread: &Pubkey,
    shard_account: &AccountInfo,
    shard_idx: u8,
) -> Result<u8, ProgramError> {
    if shard_idx >= N_AUTHOR_FEE_SHARDS {
        return Err(ProtocolError::InvalidShard.into());
    }

    let (expected, bump) = derive_author_fee_pda(program_id, thread, shard_idx);

    if *shard_account.key != expected {
        return Err(ProtocolError::InvalidPda.into());
    }

    Ok(bump)
}

/// Creates a program-owned PDA account that is robust against the
/// "create_account pre-funding" DoS. The System Program's `create_account`
/// fails if the destination already holds lamports, and PDA addresses are
/// fully predictable, so anyone can permanently brick lazy account creation by
/// sending 1 lamport to the future PDA address ahead of time.
///
/// To avoid that, when the account is already pre-funded we never call
/// `create_account`. Instead we top up the rent (if needed) with a plain
/// transfer, then `allocate` the data and `assign` ownership to the program.
/// An attacker can fund the address but cannot `allocate`/`assign` it (those
/// require the PDA's own signature, which only this program can provide via
/// `invoke_signed`), so the account is guaranteed to have empty, system-owned
/// data and this path always succeeds.
///
/// `signer_seeds` must include the bump.
pub fn create_pda_account<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    target: &AccountInfo<'a>,
    system_program_account: &AccountInfo<'a>,
    space: usize,
    signer_seeds: &[&[u8]],
) -> ProgramResult {
    // The system_instruction::* builders hard-code the System Program as the
    // CPI target, but we additionally assert the passed account is the real
    // System Program for defense-in-depth.
    assert_system_program(system_program_account)?;

    let rent = Rent::get()?;
    let required = rent.minimum_balance(space);
    let current = target.lamports();

    if current == 0 {
        invoke_signed(
            &system_instruction::create_account(
                payer.key,
                target.key,
                required,
                space as u64,
                program_id,
            ),
            &[
                payer.clone(),
                target.clone(),
                system_program_account.clone(),
            ],
            &[signer_seeds],
        )?;
        return Ok(());
    }

    // Pre-funded address: top up to rent-exemption, then take ownership.
    if current < required {
        let top_up = required - current;
        invoke(
            &system_instruction::transfer(payer.key, target.key, top_up),
            &[
                payer.clone(),
                target.clone(),
                system_program_account.clone(),
            ],
        )?;
    }

    invoke_signed(
        &system_instruction::allocate(target.key, space as u64),
        &[target.clone(), system_program_account.clone()],
        &[signer_seeds],
    )?;

    invoke_signed(
        &system_instruction::assign(target.key, program_id),
        &[target.clone(), system_program_account.clone()],
        &[signer_seeds],
    )?;

    Ok(())
}

pub fn ensure_shard_initialized<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    shard_account: &AccountInfo<'a>,
    system_program_account: &AccountInfo<'a>,
    signer_seeds: &[&[u8]],
    bump: u8,
) -> ProgramResult {
    if !is_uninitialized(shard_account) {
        return Ok(());
    }

    let mut seeds_with_bump: Vec<&[u8]> = signer_seeds.to_vec();
    let bump_slice = &[bump];

    seeds_with_bump.push(bump_slice);

    create_pda_account(
        program_id,
        payer,
        shard_account,
        system_program_account,
        0,
        &seeds_with_bump,
    )
}