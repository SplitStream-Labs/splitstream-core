# Developer guide

## Local setup

- **Rust 1.91+** — `rust-version = "1.91"` in the workspace manifest, required
  for the `wasm32v1-none` Soroban target and the MSRV declared by
  `soroban-sdk` 28.0.0.
- **Soroban target** — `rustup target add wasm32v1-none`.
- **`soroban-sdk` is pinned to `28.0.0`** in `contracts/vault/Cargo.toml`. Do
  not use release candidates. The test dependency uses the same version with the
  `testutils` feature.
- **`stellar-cli` 28.x** — pinned to the SDK major so the wasm path used
  locally matches the one used for the Testnet deployment. CI pins `28.1.0`.
  SDK 28 refuses to emit a contract artifact from a CLI older than 25.2.0, so
  a matching CLI is required, not optional.
  Install it from the release binary
  (`stellar-cli-<version>-x86_64-unknown-linux-gnu.tar.gz`) rather than
  building it.

```bash
rustup target add wasm32v1-none
cargo check --all-targets
```

## Build and test

```bash
# Tests — soroban-sdk testutils, no network needed
cargo test --workspace

# Lint
cargo clippy --all-targets -- -D warnings

# Deployable wasm
stellar contract build --package splitstream-vault
# artifact: target/wasm32v1-none/release/splitstream_vault.wasm (~20 KB)
```

CI (`.github/workflows/ci.yml`) runs `cargo check --all-targets`,
`cargo test --all-targets`, `cargo clippy --all-targets -- -D warnings` and
`stellar contract build` on every push and PR to `main`. Keep all four green.

### Do not `cargo build` for the artifact

**`cargo build` does not produce a valid Soroban wasm.** Building the crate
with plain cargo produces a host-target binary that will not deploy; the
`wasm32v1-none` path is a different build entirely, which is why CI exercises
`stellar contract build` as a separate step from `cargo test`. Always produce
the deployable artifact with `stellar contract build --package
splitstream-vault`.

The release profile in the workspace manifest (`opt-level = "z"`,
`overflow-checks = true`, `lto = true`, `codegen-units = 1`, `panic = "abort"`,
`strip = "symbols"`) is tuned for wasm size and is the profile that produces the
~20 KB artifact.

Tests use the SDK's testutils: `Env::default()`, `env.register(...)`, and
`env.mock_all_auths()` for the happy paths, with per-address `MockAuth` for the
authorization suites. The `register`/`mock_auths` APIs have changed across SDK
majors — check them against the pinned version. Tests are one module per feature
in `contracts/vault/src/test.rs`.

## Deployment

Three steps: build, deploy, initialize.

```bash
# 1. Build the artifact
stellar contract build --package splitstream-vault

# 2. Deploy it to Testnet
stellar contract deploy \
  --wasm target/wasm32v1-none/release/splitstream_vault.wasm \
  --source-account <identity> \
  --network testnet

# 3. Initialize with the admin, the oracle key, and the settlement token
stellar contract invoke \
  --id <VAULT_CONTRACT_ID> \
  --source-account <admin identity> \
  --network testnet \
  -- initialize \
    --admin <ADMIN_ADDRESS> \
    --oracle <ORACLE_ADDRESS> \
    --token CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC
```

Flag names in `stellar contract deploy`/`invoke` have moved between CLI majors —
verify them with `--help` against your pinned `stellar-cli`.

`initialize` requires the admin's authorization, so the source account must be
the admin identity. Two separate addresses are expected: `--admin` (maintainer
key, gates disputes, splits, vesting and sweeps) and `--oracle` (the CI relay
key, gates root posting and automated distributions). Neither can be changed
afterward, so get both right the first time. `initialize` is one-shot:
a second call fails with `AlreadyInitialized`.

Each deployment holds one SEP-41 token, chosen at `initialize`; a second vault
can point at a different token.

### Worked example: the current Testnet deployment

This is the record of that sequence for the live Testnet vault:

| Step | Value |
|---|---|
| Built artifact | `target/wasm32v1-none/release/splitstream_vault.wasm` (22,606 bytes) |
| Built with | `stellar-cli` 28.1.0, `soroban-sdk` 28.0.0 |
| Deployed vault | `CB456KGXH4233BHW5BUEJMN6OJ4ZMWMPP3OXJU5HVQ2T2F7A7XHWLTAT` |
| Admin passed to `initialize` | `GDTEFWNE6KIWYKU4GIVINPIIQ26E2ZMNCHCQOHFUIEFV3E4UJWVROB54` |
| Oracle passed to `initialize` | `GAW3G4BHBS37J2V3TBX4FGTIY3JXUF6YG44AAQDRV7QE2KHBEMROMJVX` |
| Token passed to `initialize` | `CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC` (native XLM SAC) |
| Network | `testnet` — Test SDF Network ; September 2015 |
| Deploy tx | https://stellar.expert/explorer/testnet/tx/05486762036ebad10fb488e657fe4efaa625a5a1438b397e86ee09727a937592 |
| `initialize` tx | https://stellar.expert/explorer/testnet/tx/fcf562b961ff3c6c240687eaf6d3546ae1c830d28abc60fa4d08f9af43262483 |
| Explorer | https://stellar.expert/explorer/testnet/contract/CB456KGXH4233BHW5BUEJMN6OJ4ZMWMPP3OXJU5HVQ2T2F7A7XHWLTAT |

Post-deploy smoke test (all against the address above):

| Call | Result |
|---|---|
| `deposit(wave-admin, 10_000_000)` | success, `deposit` event emitted |
| `get_available_funds` | `10_000_000` |
| `get_total_credited` | `0` |
| `get_vesting_reserved` | `0` |
| `get_fixed_shares` | `[]` |
| `has_claimed(1, wave-admin)` | `false` |

The earlier revision is still visible at
`CCC2LP2LOYZOLA2JW4C4K7JMR3TRJZIKHDSQYSFJ3R3MCDJLVBT3PZOC`; it predates the
solvency and TTL fixes and should not be used.

Check that the contract responds before using it:

```bash
stellar contract invoke --id <VAULT_CONTRACT_ID> --network testnet -- get_fixed_shares
```

On a fresh, initialized vault that returns an empty list. There is no view that
returns the admin, oracle or token — they live in instance storage and are only
read by the state-changing paths. If `initialize` never ran, the first
state-changing call returns `NotInitialized` (code 1).

### After deploying

- Fund the vault with `deposit` before any distribution or vesting claim, or the
  token transfers in `withdraw` and `claim_vested` will fail.
- Post the first cycle root from the oracle key with `post_cycle_root`, then run
  the manifest off-chain (splitstream-actions) so contributors can claim.
- Bookkeeping is per-entry with a persistent TTL of `535_680` ledgers (~31 days
  at 5s). Entries are only refreshed when written — see the storage layout in the
  [contract reference](contract-reference.md#storage-layout).
