# Contributing to splitstream-core

## Development setup

- Rust 1.91+ (required for the `wasm32v1-none` Soroban target, and the MSRV
  declared by `soroban-sdk` 27.0.6).
- `rustup target add wasm32v1-none` — used by `stellar contract build`.
- `stellar-cli` 27.x (matching `soroban-sdk` 27.x) for building the deployable
  wasm. Never use `cargo build` to produce the contract artifact — it does not
  produce a valid Soroban wasm.
- Tests run with plain `cargo test` using the SDK's `testutils` feature
  (`Env::default()`, `env.register(...)`, `env.mock_all_auths()`). Check the
  harness API against the pinned SDK version — `register`/`mock_auths` APIs
  have changed across majors.

## Coding standards

- **No floats, ever.** All math is integer / basis-points (`amount * bps /
  10_000`).
- **No `unwrap()` outside `#[cfg(test)]` code.** Use `?` with
  `SplitStreamError` or explicit `match`.
- **Every state-changing function emits an event** — no silent state changes.
- **Extend TTL on every persistent-storage write** in the same function that
  performs the write (`storage::bump_persistent`); bump the instance TTL on
  every admin-gated call (`storage::bump_instance`). A skipped `extend_ttl`
  silently expires accounting data — this is the most common Soroban bug.
- **Check every `require_auth()` against the correct address** (admin vs
  oracle vs contributor). Never default to `env.current_contract_address()` or
  leave test-only auth in production paths.
- **Pull-payment pattern:** the contract never iterates an unbounded
  contributor list in one transaction. The fixed-split loop is bounded by the
  small, admin-configured share list; never add a loop over the full
  contributor set.
- Naming: `snake_case` functions, `PascalCase` types, `SCREAMING_SNAKE_CASE`
  constants (e.g. `SWEEP_TIMELOCK_SECS`).
- Keep the public surface exactly as specified in the README. Do not add
  speculative functions "just in case" — file a follow-up issue instead.

## Merkle leaf format — frozen

The leaf format in `merkle.rs::leaf_hash` is part of the cross-repo contract
with splitstream-actions:

```
sha256(contributor.to_xdr(env) ++ amount.to_xdr(env))
```

Proofs use the sorted-pair convention (each pair is hashed in ascending byte
order), so left/right order never matters. Any change here breaks on-chain
verification — treat it as a breaking protocol change.

## Git workflow

- One commit per logical unit: one function, one type file, one test block.
- Never `git add .` — stage specific files only.
- Conventional commits: `type(scope): description`, e.g.
  `feat(vault): add merkle proof verification`.
- Push after every commit — never batch.

## Testing

- One test module per feature in `test.rs`: deposit/pull-payment claims,
  Merkle verification, challenge-window semantics, fixed-split/vesting/sweep
  timelocks, and per-address authorization (via `mock_auths`, not blanket
  `mock_all_auths`).
- Challenge-window tests must exercise the window boundaries (inclusive) and
  the replacement lockouts: before window closes, after claims started, second
  replacement.
- Run `cargo test --workspace` and keep `cargo clippy --all-targets` clean
  before opening a PR.