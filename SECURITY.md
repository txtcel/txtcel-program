# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in the Txtcel program, please report it
privately. **Do not open a public issue.**

- Email: contract@txtcel.com

Please include a description of the issue, reproduction steps, and the potential
impact. We will acknowledge receipt and work with you on a coordinated
disclosure timeline.

## Scope

This policy covers the on-chain program in this repository. The security contact
and policy are also embedded in the deployed binary via `solana_security_txt`.

## Design decisions and known non-issues

These items were reviewed during a manual audit against the
[`sealevel-attacks`](https://github.com/coral-xyz/sealevel-attacks) checklist.
They are documented here to make the residual risk and intended behavior
explicit; deviations from these expectations should be treated as bugs.

### L-1 — ProgramData owner check (fixed)

`assert_upgrade_authority` derives the canonical ProgramData PDA and now also
asserts the account is owned by `BPFLoaderUpgradeable` before reading the
upgrade authority from its bytes. Without the owner check a forged account at
the canonical address could spoof the upgrade authority during `init_settings`.
Covered by the regression test
`tests/audit.rs::init_settings_rejects_programdata_owned_by_non_loader`.

### L-2 — Permissionless `prepare_alloc` (accepted risk)

`prepare_alloc` is intentionally permissionless: any wallet can pre-allocate the
next page in a thread's alloc chain. A spammer can chain many empty pages and
push the thread's "current" pointer forward, degrading the read/UI experience.

This is **accepted** rather than gated on-chain because:

- Each `AllocNode` is rent-funded by the spammer and the rent is **not
  recoverable** by them, so the attack has a real, unbounded cost.
- An in-program "page must be sufficiently full before extending" gate would
  require either a write-contended counter on the shared `AllocNode` (killing
  parallel `fill_slot`) or passing N content accounts as proof (bloating every
  extend transaction). Neither trade-off is justified for a non-fund-loss,
  read-side degradation.

Mitigation is **client-side**: readers walk the chain and skip/skip-past empty
pages, and the UI does not blindly trust the raw "current" pointer.

### L-3 — Likes are not deduplicated / Sybil-resistant (by design)

A like is a **paid** action (it charges the like fee and splits it to treasury
and author). The same wallet can like the same content repeatedly, and a
sufficiently funded actor can inflate counts. This is acceptable because every
like is economically costly; like counts are a paid engagement signal, not a
unique-voter tally.

### L-5 — `reply_alloc_seq` / `reply_slot` are not validated on-chain (by design)

The reply pointer stored with content is a free-form hint. The program does not
verify that the referenced `(alloc_seq, slot)` exists or is non-empty. Resolving
and validating reply targets is a **client responsibility**; a dangling pointer
renders as an unresolved reply, not a program fault.

### L-6 — Gated thread re-opening with empty whitelist and zero entry fee (sharp edge)

When a thread's access is gated, the whitelist is empty, and the entry fee is
zero, `request_access` effectively lets anyone self-admit for free. This is the
intended consequence of that exact configuration. Thread owners that want a
closed thread must keep a non-empty whitelist and/or a non-zero entry fee; the
client surfaces this configuration so it is not entered accidentally.
