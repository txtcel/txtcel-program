# txtcel_program

On-chain Solana program for Txtcel - a native (non-Anchor) program written with
`solana-program`. Account layouts, instruction encoding and PDA seeds are kept in
sync with the TypeScript SDK [`@txtcel/protocol`](https://www.npmjs.com/package/@txtcel/protocol).

## Layout

```
src/
  lib.rs          # entrypoint + instruction dispatch + security_txt
  instruction.rs  # ProgramInstruction enum (borsh)
  state.rs        # account layouts, discriminator tags, helpers, PDA derivation
  error.rs        # custom program errors
  content/        # message body encoding (opaque, type-tagged)
  processor/      # one handler module per instruction
```

## Build

Requires the Solana toolchain (`solana-cli` with the SBF/BPF build tools).

```bash
# Build the deployable .so
cargo build-sbf

# Native unit-test build (host target, skips the on-chain entrypoint)
cargo build --features no-entrypoint
cargo test --features no-entrypoint
```

The release profile is tuned for on-chain size (`opt-level = "z"`, fat LTO).

## Deploy

```bash
solana program deploy target/deploy/txtcel_program.so
```

The program reads `program_id` at runtime, so the same binary works on any
cluster — no `declare_id!` to change.

---

## Data model overview

Txtcel models a forum/chat as a **thread** (a "channel") that owns a singly
linked list of **alloc** nodes. Each alloc node has 31 **content** slots, and
each content slot holds one message. Fees flow into sharded vault PDAs that are
later swept to the platform treasury or the thread author.

```
ThreadNode (full-address account, the channel identity)
   │
   ├─ AllocNode seq=0 ──► AllocNode seq=1 ──► AllocNode seq=2 ──► …   (linked list)
   │       │                     │
   │       └─ 31 ContentNode slots (one message each)
   │
   ├─ ThreadAccess        (optional gating: enable + entry fee + whitelist counter)
   │      └─ AccessEntry   (one PDA per allow/deny/fee-exempt wallet)
   │
   ├─ AllocLikes           (per-alloc like counters, one u32 per slot)
   └─ AuthorFee shards     (author revenue vaults, N_AUTHOR_FEE_SHARDS = 4 per thread)

ProgramSettings            (global: admin, treasury, fee bps)
TreasuryShard PDAs         (platform revenue vaults, N_TREASURY_SHARDS = 512, global)
```

### Account types & discriminator tags

Every account stores a 1-byte `tag` as its first byte so the program can
distinguish account types and reject substituted accounts.

| Tag | Const | Struct | Address kind | Purpose |
|----:|-------|--------|--------------|---------|
| 1 | `TAG_CONTENT` | `ContentNode` | PDA `[content, thread, alloc_seq, slot]` | A single message (header + opaque body). |
| 2 | `TAG_ALLOC` | `AllocNode` | PDA `[alloc, thread, alloc_seq]` | A 31-slot bucket; node in the thread's linked list. |
| 3 | `TAG_THREAD` | `ThreadNode` | Full-address (keypair) | The channel: author, title, fees, alloc list head/tail. |
| 5 | `TAG_SETTINGS` | `ProgramSettings` | PDA `[settings]` | Global admin, treasury and fee bps. |
| 6 | `TAG_ACCESS` | `ThreadAccess` | PDA `[access, thread]` | Per-thread gating config. |
| 7 | `TAG_LIKES` | `AllocLikes` | PDA `[likes, thread, alloc_seq]` | Like counts for one alloc's 31 slots. |
| 9 | `TAG_ACCESS_ENTRY` | `AccessEntry` | PDA `[acl, thread, wallet]` | One wallet's allow/deny/fee-exempt status. |

> The **thread** is intentionally **not** a PDA. It is a freshly generated
> keypair that signs its own creation. Its pubkey is the "seed" all child PDAs
> derive from. This removes any shared global counter, so channel creation is
> fully parallelizable.

### PDA seeds

| Helper | Seeds |
|--------|-------|
| `derive_settings_pda` | `["settings"]` |
| `derive_alloc_pda` | `["alloc", thread, alloc_seq: u32 le]` |
| `derive_content_pda` | `["content", thread, alloc_seq: u32 le, slot: u8]` |
| `derive_access_pda` | `["access", thread]` |
| `derive_access_entry_pda` | `["acl", thread, wallet]` |
| `derive_likes_pda` | `["likes", thread, alloc_seq: u32 le]` |
| `derive_treasury_shard_pda` | `["treasury_shard", shard: u16 le]` |
| `derive_author_fee_pda` | `["author_fee", thread, shard: u8]` |

### Key constants

| Const | Value | Meaning |
|-------|------:|---------|
| `CONTENT_SLOTS` | 31 | Message slots per alloc node. |
| `EXTEND_THRESHOLD` | 16 | Filled slots that trigger auto-extend. |
| `MAX_BODY_LEN` | 8192 | Max bytes of a message body. |
| `MAX_TITLE_LEN` | 64 | Max bytes of a thread title. |
| `N_TREASURY_SHARDS` | 512 | Global platform vault count. |
| `N_AUTHOR_FEE_SHARDS` | 4 | Per-thread author vault count. |
| `MAX_FEE_CUT_BPS` | 5000 | Cap (50%) on any admin-set platform cut. |
| `INDEX_NONE` | `u32::MAX` | "No link" sentinel for alloc pointers. |

### Fee model

- **Base fee** (platform): `base_fee_bps` of the *rent* of any newly-allocated
  account (content, root thread+alloc). Always goes to a treasury shard.
- **Message fee** (author): a fixed lamport amount set per thread
  (`ThreadNode.message_fee`), charged on every post by a non-author. Split
  between the author shard and treasury via `author_fee_cut_bps`.
- **Like fee** (author): fixed `ThreadNode.like_fee`, charged when a non-author
  likes. Split via `like_cut_bps`.
- **Entry fee** (author): `ThreadAccess.entry_fee`, charged on `RequestAccess`.
  Split via `entry_cut_bps`.

Cut bps measure the **platform's** share; the remainder goes to the author.
Fees accrue in shard PDAs and are later moved out via the `Sweep*` instructions.

### Content body (forward-compatible)

`ContentNode` = `ContentHeader` + `kind: u16` + opaque `body: Vec<u8>`. The
program never interprets `body`; it only bounds its length. New message types
get a new `kind` discriminator and are introduced **without a program upgrade**.
`kind = 0` (`KIND_TEXT`) is plain UTF-8 and is the only kind validated on-chain.

---

## Instructions

Encoding: the instruction is a Borsh-serialized `ProgramInstruction` enum
(`src/instruction.rs`). The first byte is the variant index (see table), the
rest is the variant's fields. Account order below is the exact order each
`process_*` handler reads accounts. `s` = signer, `w` = writable.

| # | Instruction | Authority | Summary |
|--:|-------------|-----------|---------|
| 0 | `CreateRootAlloc` | anyone | Create a new thread (channel) + its root alloc. |
| 1 | `FillSlot` | anyone (gated) | Post a message into a free content slot. |
| 2 | `PrepareAlloc` | anyone | Pre-create the next alloc node in the chain. |
| 3 | `SweepTreasury` | anyone | Move treasury-shard balances to the treasury wallet. |
| 4 | `SweepAuthorFees` | thread author | Move author-shard balances to the author wallet. |
| 5 | `CloseAccount` | content author | Delete a content account, reclaim rent. |
| 6 | `InitSettings` | program upgrade authority | One-time global settings init. |
| 7 | `SetTreasury` | admin | Change the treasury wallet. |
| 8 | `InitThreadAccess` | thread author | Create the thread's gating account. |
| 9 | `SetThreadAccess` | thread admin | Toggle gating on/off. |
| 10 | `AddToWhitelist` | thread admin | Allow a wallet to post in a gated thread. |
| 11 | `RemoveFromWhitelist` | thread admin | Revoke a wallet's allow entry. |
| 12 | `SetMessageFee` | thread author | Set the per-message author fee. |
| 13 | `SetBaseFee` | admin | Set the platform base fee bps. |
| 14 | `SetAuthorFeeCut` | admin | Set the platform cut of message fees. |
| 15 | `SetEntryCut` | admin | Set the platform cut of entry fees. |
| 16 | `SetLikeCut` | admin | Set the platform cut of like fees. |
| 17 | `SetLikeFee` | thread author | Set the per-like author fee. |
| 18 | `SetEntryFee` | thread admin | Set the access entry fee. |
| 19 | `RequestAccess` | anyone | Pay the entry fee to join a gated thread. |
| 20 | `LikeContent` | anyone | Like a message (increments counter, may charge). |
| 21 | `AddToBlacklist` | thread admin | Deny a wallet. |
| 22 | `RemoveFromBlacklist` | thread admin | Lift a deny entry. |
| 23 | `AppendContent` | content author | Append bytes to your own recent message. |
| 24 | `SetAdmin` | admin | Transfer global admin. |
| 25 | `AddToFeeWhitelist` | thread admin | Mark a wallet allow + fee-exempt. |
| 26 | `RemoveFromFeeWhitelist` | thread admin | Lift a fee-exempt entry. |

---

### 0. `CreateRootAlloc { message_fee: u64, treasury_shard_idx: u16, title: Vec<u8> }`

Creates a brand-new thread (channel) and its root alloc node (`alloc_seq = 0`).

**Accounts**

| # | Account | Flags | Notes |
|--:|---------|-------|-------|
| 0 | `payer` | s, w | Funds rent + base fee; becomes thread `author`. |
| 1 | `thread_account` | s, w | Fresh keypair, signs its own creation. |
| 2 | `alloc_account` | w | PDA `[alloc, thread, 0]`. |
| 3 | `settings_account` | — | `ProgramSettings` PDA (read for base fee). |
| 4 | `treasury_shard` | w | Treasury shard `treasury_shard_idx`. |
| 5 | `system_program` | — | |

**Behavior** — Validates `title` length (≤ `MAX_TITLE_LEN`), loads settings,
ensures the alloc PDA & treasury shard PDA match, asserts both new accounts are
uninitialized, lazily initializes the treasury shard, creates the thread (with
`message_fee`, `like_fee = 0`, `alloc_count = 1`) and the root alloc, then
collects the base fee on the combined rent of thread + alloc.

**Errors** — `TextTooLong`, `InvalidPda`, `InvalidShard`,
`AccountAlreadyInitialized`, `Unauthorized` (settings tag/owner mismatch).

---

### 1. `FillSlot { kind: u16, body: Vec<u8>, candidates: Vec<CandidateSlot>, extend: bool, treasury_shard_idx: u16, author_fee_shard_idx: u8, reply_alloc_seq: u32, reply_slot: u8, max_fee: u64 }`

Posts a message into the first free slot among the `candidates`, optionally
extending the alloc chain. Candidate-list + parallel slots let many posters
write to the same thread without contending on one account.

**Accounts** — fixed prefix, then a variable tail:

| # | Account | Flags | Notes |
|--:|---------|-------|-------|
| 0 | `payer` | s, w | Poster; pays fees. |
| 1 | `thread_account` | (w if `extend`) | The channel. |
| 2 | `settings_account` | — | For base fee + author cut. |
| 3 | `treasury_shard` | w | |
| 4 | `author_fee_shard` | w | |
| 5 | `system_program` | — | |
| 6..6+N | `candidate[i]` | w | One content PDA per `CandidateSlot`. |
| 6+N | `access_account` | — | `ThreadAccess` PDA (**mandatory**). |
| 7+N | `entry_account` | — | Caller's `AccessEntry` PDA (**mandatory**). |
| 8+N.. | `current_alloc`, `new_alloc` | w | Only when `extend = true`. |

**Behavior** — Bounds `body` length; runs typed validation for known kinds.
Validates shards. The `access`/`entry` PDAs are always required at fixed
positions so gating cannot be bypassed by omitting accounts:
- Gating is enforced only when the thread `enabled` **and** (`whitelist_count > 0`
  **or** `entry_fee > 0`). Blacklisted (`ACCESS_DENIED`) wallets are always
  rejected.
- The thread author posts for free; others pay `message_fee` unless their entry
  is `ACCESS_FEE_EXEMPT`.

Iterates candidates, skips already-filled ones, and into the first free slot:
creates the content PDA, writes the `ContentHeader` (with optional reply
pointers) + opaque body, computes `base_fee + author_fee`, enforces
`total_fee ≤ max_fee` (**slippage protection** against fee front-running), then
collects the base fee and the split author fee.

If `extend` and the two extra alloc accounts are present and `new_alloc` is
uninitialized, links a new alloc node onto the chain and bumps the thread's
`alloc_count` / `last_alloc_seq`.

**Errors** — `TextTooLong`, `InvalidCandidateCount`, `InvalidPda`,
`InvalidShard`, `ThreadMismatch`, `AccessDenied`, `InvalidAllocSeq`,
`InvalidSlot`, `NoFreeSlot`, `FeeExceedsMax`, `AllocAlreadyLinked`.

---

### 2. `PrepareAlloc { alloc_seq: u32 }`

Explicitly pre-creates the next alloc node (`alloc_seq + 1`) and links it. This
is the manual counterpart to `FillSlot`'s auto-extend, useful to warm capacity.

**Accounts**

| # | Account | Flags |
|--:|---------|-------|
| 0 | `payer` | s, w |
| 1 | `current_alloc_account` | w |
| 2 | `new_alloc_account` | w |
| 3 | `thread_account` | w |
| 4 | `system_program` | — |

**Behavior** — Loads the current alloc, checks `alloc_seq` matches and that it
is not already linked, creates the new alloc PDA, links
`current.next_alloc_seq → new_seq`, updates the thread's `alloc_count` /
`last_alloc_seq`.

**Errors** — `ThreadMismatch`, `InvalidAllocSeq`, `AllocAlreadyLinked`,
`InvalidPda`, `AccountAlreadyInitialized`.

---

### 3. `SweepTreasury { shard_indices: Vec<u16> }`

Moves the excess (above rent-exempt minimum) from each listed treasury shard
into the treasury wallet. Permissionless: anyone may trigger it; funds can only
go to `settings.treasury`.

**Accounts** — `[settings, treasury_wallet (w), ...shards (w)]`. There must be
exactly one shard account per index in `shard_indices` and the order must match.

**Errors** — `InvalidTreasury`, `NothingToSweep`, `InvalidShard`, `InvalidPda`,
`AccountOwnerMismatch`.

---

### 4. `SweepAuthorFees { shard_indices: Vec<u8> }`

Like `SweepTreasury` but for a thread's author-fee shards; only the thread
author (signer) can collect.

**Accounts** — `[thread_account, author_wallet (s, w), ...shards (w)]`, one
shard per index.

**Errors** — `InvalidAuthor`, `MissingSigner`, `NothingToSweep`, `InvalidShard`.

---

### 5. `CloseAccount`

Closes a program-owned content account and refunds its rent to the signer.
Currently only `TAG_CONTENT` accounts are closeable.

**Accounts**

| # | Account | Flags | Notes |
|--:|---------|-------|-------|
| 0 | `payer` | s | Must be the content author; receives rent. |
| 1 | `target_account` | w | The content account to close. |
| 2 | `likes_account` | w, optional | If supplied, the freed slot's like counter is reset to 0. |

**Behavior** — Verifies authorship, optionally zeroes the slot's like count,
transfers all lamports out, zeroes + shrinks data, and reassigns the account to
the System Program (so it can't be "revived" with a stale tag in-tx).

**Errors** — `InvalidTag`, `Unauthorized`, `InvalidPda`, `InvalidAllocSeq`.

---

### 6. `InitSettings { treasury: Pubkey }`

One-time creation of the global `ProgramSettings`. Callable **only** by the
program's upgrade authority (verified against the BPF loader's ProgramData
account). Initializes all four fee bps to `1000` (10%).

**Accounts** — `[authority (s, w), settings (w), programdata, system_program]`.

**Errors** — `Unauthorized`, `InvalidPda`, `AccountAlreadyInitialized`.

---

### 7. `SetTreasury { treasury: Pubkey }`

Admin-only. Updates `ProgramSettings.treasury`.
**Accounts** — `[authority (s), settings (w)]`. **Errors** — `Unauthorized`.

---

### 8. `InitThreadAccess { enabled: bool, treasury_shard_idx: u16 }`

Thread-author-only. Creates the `ThreadAccess` PDA that holds gating config
(`enabled`, `entry_fee = 0`, `whitelist_count = 0`, `admin = author`). The
author pays the account's rent into the treasury shard.

**Accounts** — `[authority (s, w), thread, access (w), treasury_shard (w), system_program]`.

**Errors** — `Unauthorized`, `InvalidPda`, `AccountAlreadyInitialized`,
`InvalidShard`.

---

### 9. `SetThreadAccess { enabled: bool }`

Thread-admin-only. Toggles the `enabled` flag.
**Accounts** — `[authority (s), access (w)]`. **Errors** — `Unauthorized`,
`InvalidPda`.

---

### 10. `AddToWhitelist { wallet: Pubkey }`

Thread-admin-only. Creates or flips the wallet's `AccessEntry` to
`ACCESS_ALLOWED` and bumps `whitelist_count` if it wasn't already an allow
entry.

**Accounts** — `[authority (s, w), access (w), entry (w), system_program]`.
The `entry` is the `[acl, thread, wallet]` PDA. **Errors** — `Unauthorized`,
`InvalidPda`, `ThreadMismatch`.

---

### 11. `RemoveFromWhitelist { wallet: Pubkey }`

Thread-admin-only. Closes the wallet's `ACCESS_ALLOWED` entry (refunds rent to
the admin) and decrements `whitelist_count`.
**Accounts** — same as #10. **Errors** — `AccessListMissing`, `Unauthorized`,
`InvalidPda`, `ThreadMismatch`.

---

### 12. `SetMessageFee { fee: u64 }`

Thread-author-only. Sets the fixed per-message author fee (lamports).
**Accounts** — `[authority (s), thread (w)]`. **Errors** — `Unauthorized`.

---

### 13–16. Platform fee setters (admin-only)

All four share the layout `[authority (s), settings (w)]`, take a single
`fee_bps: u32`, and reject values `> MAX_FEE_CUT_BPS` (5000).

| # | Instruction | Field updated | Meaning |
|--:|-------------|---------------|---------|
| 13 | `SetBaseFee { fee_bps }` | `base_fee_bps` | Platform % of new-account rent. |
| 14 | `SetAuthorFeeCut { fee_bps }` | `author_fee_cut_bps` | Platform % of message fees. |
| 15 | `SetEntryCut { fee_bps }` | `entry_cut_bps` | Platform % of entry fees. |
| 16 | `SetLikeCut { fee_bps }` | `like_cut_bps` | Platform % of like fees. |

**Errors** — `InvalidFeeBps`, `Unauthorized`.

---

### 17. `SetLikeFee { fee: u64 }`

Thread-author-only. Sets the fixed per-like author fee.
**Accounts** — `[authority (s), thread (w)]`. **Errors** — `Unauthorized`.

---

### 18. `SetEntryFee { fee: u64 }`

Thread-admin-only. Sets `ThreadAccess.entry_fee` (lamports charged on
`RequestAccess`).
**Accounts** — `[authority (s), access (w)]`. **Errors** — `Unauthorized`,
`InvalidPda`.

---

### 19. `RequestAccess { treasury_shard_idx: u16, author_fee_shard_idx: u8 }`

A wallet pays the thread's `entry_fee` to gain access, creating its own
`ACCESS_ALLOWED` entry. The fee is split between author and treasury via
`entry_cut_bps`; `whitelist_count` is incremented.

**Accounts** — `[payer (s, w), access (w), entry (w), thread, settings, treasury_shard (w), author_fee_shard (w), system_program]`.

**Behavior** — Rejects when `entry_fee == 0` (`ZeroEntryFee`), when the wallet is
blacklisted (`AccessDenied`), or already a member (`AccessListDuplicate`).

**Errors** — `ZeroEntryFee`, `AccessDenied`, `AccessListDuplicate`,
`ThreadMismatch`, `InvalidPda`, `InvalidShard`.

---

### 20. `LikeContent { alloc_seq: u32, slot: u8, treasury_shard_idx: u16, author_fee_shard_idx: u8, max_fee: u64 }`

Increments the like counter for a message slot, lazily creating the `AllocLikes`
account. If the thread has a `like_fee > 0` and the liker isn't the content
author, the fee is split between author and treasury via `like_cut_bps`, capped
by `max_fee` (slippage protection).

**Accounts** — `[payer (s, w), likes (w), content, thread, settings, treasury_shard (w), author_fee_shard (w), system_program]`.

**Errors** — `InvalidSlot`, `InvalidPda`, `InvalidShard`, `FeeExceedsMax`,
`InvalidTag`.

---

### 21. `AddToBlacklist { wallet: Pubkey }`

Thread-admin-only. Creates or flips the wallet's entry to `ACCESS_DENIED`. If
the wallet was an allow entry, `whitelist_count` is decremented.
**Accounts** — `[authority (s, w), access (w), entry (w), system_program]`.

---

### 22. `RemoveFromBlacklist { wallet: Pubkey }`

Thread-admin-only. Closes the wallet's `ACCESS_DENIED` entry (refunds rent).
**Accounts** — same as #21. **Errors** — `AccessListMissing`, `Unauthorized`.

---

### 23. `AppendContent { chunk: Vec<u8>, treasury_shard_idx: u16, author_fee_shard_idx: u8 }`

Appends raw bytes to the author's own existing content body (used to send a
message larger than one transaction). Only the original author can append, and
only within a **120-second window** of the message's `created_at`. Only the
incremental base fee (on the rent delta) is charged — the per-message author fee
is **not** re-charged.

**Accounts** — `[payer (s, w), content (w), thread, settings, treasury_shard (w), author_fee_shard (w), system_program]`.

**Errors** — `Unauthorized`, `ThreadMismatch`, `AppendWindowExpired`,
`TextTooLong`, `InvalidPda`, `InvalidShard`.

---

### 24. `SetAdmin { new_admin: Pubkey }`

Admin-only. Transfers global admin in `ProgramSettings`.
**Accounts** — `[authority (s), settings (w)]`. **Errors** — `Unauthorized`.

---

### 25. `AddToFeeWhitelist { wallet: Pubkey }`

Thread-admin-only. Sets the wallet's entry to `ACCESS_FEE_EXEMPT` — implicitly
allowed to post **and** exempt from the per-message author fee. Moving a plain
allow entry here decrements `whitelist_count` (fee-exempt members are tracked
separately).
**Accounts** — `[authority (s, w), access (w), entry (w), system_program]`.

---

### 26. `RemoveFromFeeWhitelist { wallet: Pubkey }`

Thread-admin-only. Closes a wallet's `ACCESS_FEE_EXEMPT` entry (only entries in
that exact state). **Accounts** — same as #25. **Errors** — `AccessListMissing`.

---

## Errors

`ProtocolError` (`src/error.rs`) is returned as `ProgramError::Custom(code)`.

| Code | Variant | Meaning |
|-----:|---------|---------|
| 0 | `MissingSigner` | A required signer didn't sign. |
| 1 | `NotWritable` | A required-writable account wasn't writable. |
| 2 | `InvalidTag` | Account discriminator tag mismatch. |
| 3 | `TextTooLong` | Body/title exceeds the max length. |
| 4 | `AllocAlreadyLinked` | Alloc node already has a `next`. |
| 5 | `SlotAlreadyUsed` | Slot already filled. |
| 6 | `InvalidCandidateCount` | Empty/incorrect candidate set. |
| 7 | `AccountOwnerMismatch` | Account not owned by the program. |
| 8 | `AccountAlreadyInitialized` | Target account already exists. |
| 9 | `InvalidAccountData` | Deserialization / arithmetic failure. |
| 10 | `NoFreeSlot` | No candidate slot was free. |
| 11 | `InvalidShard` | Shard index/PDA invalid. |
| 12 | `ThreadMismatch` | Account belongs to a different thread. |
| 13 | `InvalidPda` | Supplied address ≠ derived PDA. |
| 14 | `Unauthorized` | Caller is not admin/author. |
| 15 | `InvalidTreasury` | Treasury wallet ≠ settings treasury. |
| 16 | `InvalidProgramAccount` | Wrong program account. |
| 17 | `AccessDenied` | Wallet blacklisted or not allowed. |
| 18 | `AccessListFull` | (reserved) |
| 19 | `AccessListDuplicate` | Wallet already a member. |
| 20 | `AccessListMissing` | Entry missing / wrong status to close. |
| 21 | `MissingAccessAccount` | Required access account absent. |
| 22 | `InvalidFeeBps` | Fee bps > `MAX_FEE_CUT_BPS`. |
| 23 | `InvalidAuthor` | Author wallet ≠ thread author. |
| 25 | `ZeroEntryFee` | `RequestAccess` with no entry fee set. |
| 26 | `InvalidSlot` | Slot index ≥ `CONTENT_SLOTS`. |
| 27 | `AccessListConflict` | (reserved) |
| 28 | `InvalidAllocSeq` | Bad/mismatched alloc sequence. |
| 29 | `NothingToSweep` | No shards supplied to a sweep. |
| 30 | `AppendWindowExpired` | Append after the 120s window. |
| 31 | `FeeExceedsMax` | Computed fee > caller's `max_fee`. |

## Security

See [`SECURITY.md`](./SECURITY.md). Security contact and policy are also embedded
in the binary via `solana_security_txt` (see `lib.rs`).

Notable hardening:

- **PDA pre-funding DoS resistance** — `create_pda_account` tops-up + `allocate`
  + `assign` when a PDA address was pre-funded, instead of failing on
  `create_account`.
- **Slippage caps** — `FillSlot` / `LikeContent` take `max_fee` so authors/admins
  can't front-run fee hikes.
- **Mandatory access accounts** — gating PDAs are at fixed positions and derived
  by the program, so they can't be omitted to bypass checks.
- **Safe close** — closed accounts are zeroed, shrunk and re-assigned to the
  System Program to prevent in-tx revival.

## License

[MIT](./LICENSE)
