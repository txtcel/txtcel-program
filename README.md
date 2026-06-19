# txtcel_program

On-chain Solana program for Txtcel — a native (non-Anchor) program written with
`solana-program`. Account layouts, instruction encoding and PDA seeds are kept in
sync with the TypeScript SDK [`@txtcel/protocol`](https://www.npmjs.com/package/@txtcel/protocol).

## Layout

```
src/
  lib.rs          # entrypoint + instruction dispatch + security_txt
  instruction.rs  # ProgramInstruction enum (borsh)
  state.rs        # account layouts and discriminator tags
  error.rs        # custom program errors
  content/        # message body encoding
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

## Security

See [`SECURITY.md`](./SECURITY.md). Security contact and policy are also embedded
in the binary via `solana_security_txt` (see `lib.rs`).

## License

[MIT](./LICENSE)
