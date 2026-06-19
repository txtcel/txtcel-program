use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

use crate::state::CandidateSlot;

#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub enum ProgramInstruction {
    CreateRootAlloc { message_fee: u64, treasury_shard_idx: u16, title: Vec<u8> },
    FillSlot { kind: u16, body: Vec<u8>, candidates: Vec<CandidateSlot>, extend: bool, treasury_shard_idx: u16, author_fee_shard_idx: u8, reply_alloc_seq: u32, reply_slot: u8, max_fee: u64 },
    PrepareAlloc { alloc_seq: u32 },
    SweepTreasury { shard_indices: Vec<u16> },
    SweepAuthorFees { shard_indices: Vec<u8> },
    CloseAccount,
    InitSettings { treasury: Pubkey },
    SetTreasury { treasury: Pubkey },
    InitThreadAccess { enabled: bool, treasury_shard_idx: u16 },
    SetThreadAccess { enabled: bool },
    AddToWhitelist { wallet: Pubkey },
    RemoveFromWhitelist { wallet: Pubkey },
    SetMessageFee { fee: u64 },
    SetBaseFee { fee_bps: u32 },
    SetAuthorFeeCut { fee_bps: u32 },
    SetEntryCut { fee_bps: u32 },
    SetLikeCut { fee_bps: u32 },
    SetLikeFee { fee: u64 },
    SetEntryFee { fee: u64 },
    RequestAccess { treasury_shard_idx: u16, author_fee_shard_idx: u8 },
    LikeContent { alloc_seq: u32, slot: u8, treasury_shard_idx: u16, author_fee_shard_idx: u8, max_fee: u64 },
    AddToBlacklist { wallet: Pubkey },
    RemoveFromBlacklist { wallet: Pubkey },
    AppendContent { chunk: Vec<u8>, treasury_shard_idx: u16, author_fee_shard_idx: u8 },
    SetAdmin { new_admin: Pubkey },
    AddToFeeWhitelist { wallet: Pubkey },
    RemoveFromFeeWhitelist { wallet: Pubkey },
}
