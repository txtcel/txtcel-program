use solana_program::program_error::ProgramError;

/// Program-specific error codes returned to clients as `ProgramError::Custom`.
///
/// The explicit `#[repr(u32)]` discriminants are the stable on-chain error
/// codes clients decode, so existing values must never be reused or shifted
/// (note `24` is intentionally retired). New errors are appended at the end.
#[repr(u32)]
pub enum ProtocolError {
    /// A required account did not sign the transaction.
    MissingSigner = 0,
    /// An account expected to be mutable was passed read-only.
    NotWritable = 1,
    /// An account's leading tag byte did not match the expected type.
    InvalidTag = 2,
    /// A text/body payload exceeded its maximum allowed length.
    TextTooLong = 3,
    /// Retired guard: alloc nodes no longer store forward links, so this is no
    /// longer produced (a non-tail extend now returns `InvalidAllocSeq`). Kept
    /// in place to preserve the stable on-chain discriminant of every variant
    /// after it; do not reuse or renumber.
    AllocAlreadyLinked = 4,
    /// The targeted content slot is already occupied.
    SlotAlreadyUsed = 5,
    /// The candidate slot list was empty or otherwise the wrong size.
    InvalidCandidateCount = 6,
    /// An account is not owned by the program it was expected to belong to.
    AccountOwnerMismatch = 7,
    /// An account expected to be empty was already initialized.
    AccountAlreadyInitialized = 8,
    /// Account bytes failed to deserialize or violated an invariant.
    InvalidAccountData = 9,
    /// None of the supplied candidate slots was free to fill.
    NoFreeSlot = 10,
    /// A shard index or shard account was invalid for the operation.
    InvalidShard = 11,
    /// An account referenced a different thread than expected.
    ThreadMismatch = 12,
    /// An account's address did not match its derived PDA.
    InvalidPda = 13,
    /// The signer lacked the required authority for the action.
    Unauthorized = 14,
    /// The provided treasury wallet did not match program settings.
    InvalidTreasury = 15,
    /// A required program/sysvar account was not the expected one.
    InvalidProgramAccount = 16,
    /// The wallet is blacklisted or not permitted in a gated thread.
    AccessDenied = 17,
    /// The access list cannot accept more members.
    AccessListFull = 18,
    /// The wallet already has an equivalent access entry.
    AccessListDuplicate = 19,
    /// The expected access entry was missing.
    AccessListMissing = 20,
    /// A mandatory access-control account was not supplied.
    MissingAccessAccount = 21,
    /// A basis-points value exceeded its allowed maximum.
    InvalidFeeBps = 22,
    /// The provided author wallet did not match the thread's author.
    InvalidAuthor = 23,
    /// An entry fee of zero was set where a positive fee is required.
    ZeroEntryFee = 25,
    /// The slot index was out of range for an alloc.
    InvalidSlot = 26,
    /// The requested access change conflicts with the entry's current state.
    AccessListConflict = 27,
    /// The alloc sequence was out of range or did not match the tail.
    InvalidAllocSeq = 28,
    /// A sweep found no shards or no excess lamports to move.
    NothingToSweep = 29,
    /// The append time window for the content slot has elapsed.
    AppendWindowExpired = 30,
    /// The computed fee exceeded the caller's `max_fee` slippage cap.
    FeeExceedsMax = 31,
    /// The caller already follows the channel.
    AlreadyFollowing = 32,
    /// The caller is not following the channel.
    NotFollowing = 33,
    /// The caller's follow registry is at capacity.
    FollowListFull = 34,
}

impl From<ProtocolError> for ProgramError {
    /// Converts a protocol error into Solana's `ProgramError::Custom`.
    ///
    /// # Parameters
    /// - `value` — the protocol error to surface to the caller.
    ///
    /// # Returns
    /// - `ProgramError::Custom` carrying the error's `u32` discriminant.
    fn from(value: ProtocolError) -> Self {
        ProgramError::Custom(value as u32)
    }
}
