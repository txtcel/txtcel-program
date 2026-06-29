pub mod content;
pub mod error;
pub mod instruction;
pub mod processor;
pub mod state;

use borsh::BorshDeserialize;
use solana_program::{
    account_info::AccountInfo,
    entrypoint,
    entrypoint::ProgramResult,
    program_error::ProgramError,
    pubkey::Pubkey,
};

use instruction::ProgramInstruction;
use processor::*;

entrypoint!(process_instruction);

#[cfg(not(feature = "no-entrypoint"))]
solana_security_txt::security_txt! {
    name: "Txtcel",
    project_url: "https://txtcel.com",
    contacts: "email:contract@txtcel.com",
    policy: "https://github.com/txtcel/txtcel/blob/main/SECURITY.md",
    preferred_languages: "en",
    source_code: "https://github.com/txtcel/txtcel-protocol"
}

/// Program entrypoint: decodes the raw instruction data and dispatches to the
/// matching `process_*` handler.
///
/// This is the single Borsh-decode + `match` seam between Solana's untyped byte
/// interface and the typed handlers; every supported instruction is routed from
/// exactly one arm here, which keeps argument forwarding in one auditable place.
///
/// # Parameters
/// - `program_id` — address this program is deployed at; forwarded to every
///   handler for PDA derivation and account-ownership checks.
/// - `accounts` — accounts supplied by the caller, in the order the selected
///   handler expects; their validation is delegated to that handler.
/// - `instruction_data` — Borsh-encoded `ProgramInstruction` that selects the
///   handler and carries its arguments.
///
/// # Returns
/// - `Ok(())` when the dispatched handler succeeds.
/// - `ProgramError::InvalidInstructionData` if the bytes do not decode to a
///   known instruction, otherwise whatever error the handler returns.
pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    let instruction = ProgramInstruction::try_from_slice(instruction_data)
        .map_err(|_| ProgramError::InvalidInstructionData)?;
    match instruction {
        ProgramInstruction::CreateRootAlloc { message_fee, treasury_shard_idx, title } => {
            process_create_root_alloc(program_id, accounts, message_fee, treasury_shard_idx, title)
        }
        ProgramInstruction::FillSlot { kind, body, candidates, treasury_shard_idx, author_fee_shard_idx, reply_alloc_seq, reply_slot, max_fee } => {
            process_fill_slot(program_id, accounts, kind, body, candidates, treasury_shard_idx, author_fee_shard_idx, reply_alloc_seq, reply_slot, max_fee)
        }
        ProgramInstruction::PrepareAlloc { alloc_seq } => {
            process_prepare_alloc(program_id, accounts, alloc_seq)
        }
        ProgramInstruction::SweepTreasury { shard_indices } => process_sweep_treasury(program_id, accounts, shard_indices),
        ProgramInstruction::SweepAuthorFees { shard_indices } => process_sweep_author_fees(program_id, accounts, shard_indices),
        ProgramInstruction::CloseAccount => process_close_account(program_id, accounts),
        ProgramInstruction::InitSettings { treasury } => {
            process_init_settings(program_id, accounts, treasury)
        }
        ProgramInstruction::SetTreasury { treasury } => {
            process_set_treasury(program_id, accounts, treasury)
        }
        ProgramInstruction::InitThreadAccess { enabled, treasury_shard_idx } => {
            process_init_thread_access(program_id, accounts, enabled, treasury_shard_idx)
        }
        ProgramInstruction::SetThreadAccess { enabled } => {
            process_set_thread_access(program_id, accounts, enabled)
        }
        ProgramInstruction::AddToWhitelist { wallet } => {
            process_add_to_whitelist(program_id, accounts, wallet)
        }
        ProgramInstruction::RemoveFromWhitelist { wallet } => {
            process_remove_from_whitelist(program_id, accounts, wallet)
        }
        ProgramInstruction::SetMessageFee { fee } => {
            process_set_message_fee(program_id, accounts, fee)
        }
        ProgramInstruction::SetBaseFee { fee_bps } => {
            process_set_base_fee(program_id, accounts, fee_bps)
        }
        ProgramInstruction::SetAuthorFeeCut { fee_bps } => {
            process_set_author_fee_cut(program_id, accounts, fee_bps)
        }
        ProgramInstruction::SetEntryCut { fee_bps } => {
            process_set_entry_cut(program_id, accounts, fee_bps)
        }
        ProgramInstruction::SetLikeCut { fee_bps } => {
            process_set_like_cut(program_id, accounts, fee_bps)
        }
        ProgramInstruction::SetLikeFee { fee } => {
            process_set_like_fee(program_id, accounts, fee)
        }
        ProgramInstruction::SetEntryFee { fee } => {
            process_set_entry_fee(program_id, accounts, fee)
        }
        ProgramInstruction::RequestAccess { treasury_shard_idx, author_fee_shard_idx } => {
            process_request_access(program_id, accounts, treasury_shard_idx, author_fee_shard_idx)
        }
        ProgramInstruction::LikeContent { alloc_seq, slot, treasury_shard_idx, author_fee_shard_idx, max_fee } => {
            process_like_content(program_id, accounts, alloc_seq, slot, treasury_shard_idx, author_fee_shard_idx, max_fee)
        }
        ProgramInstruction::AddToBlacklist { wallet } => {
            process_add_to_blacklist(program_id, accounts, wallet)
        }
        ProgramInstruction::RemoveFromBlacklist { wallet } => {
            process_remove_from_blacklist(program_id, accounts, wallet)
        }
        ProgramInstruction::AppendContent { chunk, treasury_shard_idx, author_fee_shard_idx } => {
            process_append_content(program_id, accounts, chunk, treasury_shard_idx, author_fee_shard_idx)
        }
        ProgramInstruction::SetAdmin { new_admin } => {
            process_set_admin(program_id, accounts, new_admin)
        }
        ProgramInstruction::AddToFeeWhitelist { wallet } => {
            process_add_to_fee_whitelist(program_id, accounts, wallet)
        }
        ProgramInstruction::RemoveFromFeeWhitelist { wallet } => {
            process_remove_from_fee_whitelist(program_id, accounts, wallet)
        }
        ProgramInstruction::Subscribe => process_subscribe(program_id, accounts),
        ProgramInstruction::Unsubscribe => process_unsubscribe(program_id, accounts),
    }
}
