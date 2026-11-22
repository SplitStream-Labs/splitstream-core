# Changelog

All notable changes to `splitstream-core` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Security

- **Sweeps are capped at free funds.** `execute_sweep` now rejects any amount
  above `balance(vault) - CreditedTotal - VestingReserved` with the new
  `InsufficientFreeFunds` (18) error. Previously a 72-hour timelock only delayed
  an admin sweep that would strand contributors' credited balances and unvested
  schedule remaining.
- **Per-cycle over-payment is impossible.** `credit_claim` tracks
  `CycleClaimedTotal(cycle_id)` and rejects a claim that would push cumulative
  claims above the posted `total_amount` with the new `CycleTotalExceeded` (19)
  error.
- **Vesting underflow fixed.** `claim_vested` computes `vested - claimed`
  saturating, so a schedule re-created with a smaller total can no longer
  underflow (the release profile enables `overflow-checks`, so this was a trap)
  — and `create_vesting` now rejects a `total` below the already-claimed amount
  outright.
- **Instance TTL no longer lapses.** Every state-changing entry point
  (`deposit`, `post_cycle_root`, `credit_claim`, `withdraw`, the two fixed-split
  distributions, `claim_vested`) bumps the instance TTL. `Admin`, `Oracle` and
  `Token` live in instance storage, so a lapsed instance made the whole contract
  unreachable.
- `initialize` checks whether it has already run *before* `require_auth`, so a
  second call reports `AlreadyInitialized` rather than trapping on a different
  key.

### Added

- Redeployed the hardened contract to Testnet at
  `CB456KGXH4233BHW5BUEJMN6OJ4ZMWMPP3OXJU5HVQ2T2F7A7XHWLTAT` (admin
  `GDTEFWNE6KIWYKU4GIVINPIIQ26E2ZMNCHCQOHFUIEFV3E4UJWVROB54`, oracle
  `GAW3G4BHBS37J2V3TBX4FGTIY3JXUF6YG44AAQDRV7QE2KHBEMROMJVX`, native XLM SAC),
  built with `stellar-cli` 28.1.0 and smoke-tested with a deposit plus the
  read-only views. Supersedes
  `CCC2LP2LOYZOLA2JW4C4K7JMR3TRJZIKHDSQYSFJ3R3MCDJLVBT3PZOC`.
- Deterministic property/fuzz suite (`contracts/vault/src/property_test.rs`):
  random manifests (1–32 leaves) for proof soundness and tamper rejection, plus
  randomised vesting walks asserting the linear formula, monotonicity, the
  `total` ceiling and the vault accounting identity. Suite: 44 tests.
- Liability ledger: `CreditedTotal` and `VestingReserved` in instance storage,
  maintained transactionally with every balance write, plus the read-only views
  `get_total_credited`, `get_vesting_reserved` and `get_available_funds`.
- Three solvency regression tests (`test::solvency`) and a vesting-recreation
  guard test; the suite is now 40 tests.

- Unified documentation site (`mkdocs.yml`, `docs-hub/`, `scripts/assemble_docs.py`)
  that merges the docs of `splitstream-core`, `splitstream-actions`, and
  `splitstream-sdk-cli` into one GitHub Pages site, deployed by
  `.github/workflows/docs.yml`.
- Cross-repo [architecture page](docs-hub/architecture.md) documenting the three
  frozen interfaces (Merkle leaf format, payout-manifest schema, contract ABI).
- Issue templates, a PR template with the vault non-negotiables checklist,
  `CODEOWNERS` for protocol-critical files, and Dependabot configuration.
- `rust-toolchain.toml` and `.editorconfig`; `CHANGELOG.md`.

### Changed

- Dependencies and tooling aligned on `soroban-sdk` 28.0.0 and `stellar-cli`
  28.1.0 (CI previously pinned CLI 27.1.0 while Cargo resolved SDK 28.0.0, and
  the docs still advertised 27.0.6). SDK 28 refuses to emit a contract artifact
  from an older CLI.
- Documentation links now point at the unified GitHub Pages site instead of the
  per-repo GitBook spaces.
- Corrected the declared minimum Rust version from 1.84 to 1.91, the MSRV
  declared by `soroban-sdk` 27.0.6, in `Cargo.toml` and the docs.
- Repository URLs updated from the personal account to the `SplitStream-Labs`
  organisation.

## [0.1.0] - 2026-09-08

### Added

- `SplitStreamVault` Soroban contract: one-time `initialize`, `deposit`, and
  read-only views (`get_balance`, `get_cycle_info`, `has_claimed`,
  `get_vesting`, `get_fixed_shares`).
- Merkle-proof claims: `post_cycle_root`, `challenge_and_replace_root`, and
  `credit_claim` with the 24-hour challenge window, single-replacement rule, and
  `claims_started` lockout; `withdraw` as a checks-effects-interactions pull
  payment.
- Fixed basis-point splits: `configure_fixed_shares`, `distribute_fixed`
  (oracle), and `admin_distribute_fixed` (admin) over a shared internal body.
- Linear vesting: `create_vesting` and `claim_vested` (with a dedicated
  `NoVestingSchedule` error).
- Timelocked treasury sweeps: `request_sweep`, `execute_sweep`, `cancel_sweep`
  behind a 72-hour timelock.
- Persistent-storage TTL management (`storage::bump_persistent`) and instance
  TTL bumps on admin-gated calls.
- Test suite covering deposits and pull-payment claims, Merkle verification
  (including golden fixtures), challenge-window boundaries and lockouts,
  fixed-split/vesting/sweep timelocks, and per-address authorization.
- CI workflow running `cargo check`, `cargo test`, `cargo clippy -D warnings`,
  and `stellar contract build`.

[Unreleased]: https://github.com/SplitStream-Labs/splitstream-core/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/SplitStream-Labs/splitstream-core/releases/tag/v0.1.0
