use solana_program::program_error::ProgramError;

#[repr(u32)]
pub enum ProtocolError {
    MissingSigner = 0,
    NotWritable = 1,
    InvalidTag = 2,
    TextTooLong = 3,
    AllocAlreadyLinked = 4,
    SlotAlreadyUsed = 5,
    InvalidCandidateCount = 6,
    AccountOwnerMismatch = 7,
    AccountAlreadyInitialized = 8,
    InvalidAccountData = 9,
    NoFreeSlot = 10,
    InvalidShard = 11,
    ThreadMismatch = 12,
    InvalidPda = 13,
    Unauthorized = 14,
    InvalidTreasury = 15,
    InvalidProgramAccount = 16,
    AccessDenied = 17,
    AccessListFull = 18,
    AccessListDuplicate = 19,
    AccessListMissing = 20,
    MissingAccessAccount = 21,
    InvalidFeeBps = 22,
    InvalidAuthor = 23,
    ZeroEntryFee = 25,
    InvalidSlot = 26,
    AccessListConflict = 27,
    InvalidAllocSeq = 28,
    NothingToSweep = 29,
    AppendWindowExpired = 30,
    FeeExceedsMax = 31,
}

impl From<ProtocolError> for ProgramError {
    fn from(value: ProtocolError) -> Self {
        ProgramError::Custom(value as u32)
    }
}
