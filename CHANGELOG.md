# Changelog

All notable changes to `splitstream-core` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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
