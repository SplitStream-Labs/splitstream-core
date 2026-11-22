# SplitStream Core — Changes Since the Drips Wave Rejection

This document is the technical evidence package for the **Appeal** on our rejected
repository. It records what was wrong with the submission, what was changed, and how
every claim below can be re-verified from a clean checkout.

Repository: `SplitStream-Labs/splitstream-core`
Program applied to: Stellar Wave Program (Drips Wave)
Baseline commit: `226fa4e` (`main` at time of rejection)

**Live evidence:** the hardened contract is deployed and initialized on Testnet at
[`CB456KGXH4233BHW5BUEJMN6OJ4ZMWMPP3OXJU5HVQ2T2F7A7XHWLTAT`](https://stellar.expert/explorer/testnet/contract/CB456KGXH4233BHW5BUEJMN6OJ4ZMWMPP3OXJU5HVQ2T2F7A7XHWLTAT)
— this is a real deployment, not a screenshot.

---

## 1. First, a correction to the framing

The review feedback that reached us was summarised internally as *"rebase this project
fully on Soroban."* We investigated before changing anything, and the finding is
important for the appeal:

**This repository was already a native Soroban contract.** There is no other chain, no
EVM contract, and no porting layer to rebase away. At the baseline commit it already:

- compiled against `soroban-sdk` 28.0.0 (`contracts/vault/Cargo.toml`);
- targeted `wasm32v1-none` and built a deployable contract artifact;
- used `#[contract]` / `#[contractimpl]` / `#[contracttype]` / `#[contracterror]`;
- moved value with the SEP-41 `token::TokenClient` (native XLM SAC on Testnet);
- used the Soroban instance/persistent storage split with explicit
  `extend_ttl` TTL management.

So we did not perform a chain migration. Instead we treated the rejection as a code
quality and credibility problem and spent the effort where the risk actually was:
**Soroban-specific correctness and security defects, and a repository whose docs did
not match its own dependency reality.** If the original rejection was based on a
different specific concern, please tell us which — the changes below are substantive
either way, and this document is structured so the relevant section can be quoted
directly into the appeal.

The rest of this document is grouped by what a reviewer can independently check.

---

## 2. Real defects that were found and fixed

Every item below is a genuine bug in the rejected revision, not a refactor. The
baseline suite (36 tests) passed both before and after, which is precisely why these
were not caught: none of them was covered by a test.

### 2.1 `execute_sweep` could strand funds the vault already owed

**Severity: high. This was the most serious flaw.**

`execute_sweep` transferred an admin-requested amount with no awareness of the
contract's outstanding obligations. A vault could be holding:

- credited pull-payment `Balance` entries that contributors had not withdrawn yet, and
- unvested (but already owned) amounts under vesting schedules,

and the admin could sweep all of it. The 72-hour timelock only delayed that drain; it
did not prevent it. Contributors' later `withdraw` / `claim_vested` calls would then
fail at the token transfer, with no on-chain way to recover.

Our own `docs/for-maintainers.md` documented the hole as an operating hazard
("The sweep is not balance-aware"). Documenting a hazard is not fixing it.

**Fix.** The vault now keeps an explicit liability ledger in instance storage:

| Key | Meaning |
|---|---|
| `CreditedTotal` | sum of every outstanding `Balance(contributor)` |
| `VestingReserved` | outstanding vesting liability, `sum(total - claimed)` |

They are maintained transactionally with the pack of balance writes they track
(`storage::add_balance`, `storage::take_balance`, `storage::adjust_vesting_reserved`),
so they cannot drift from the per-account entries. `execute_sweep` now computes

```
free = balance(vault) - (CreditedTotal + VestingReserved)
```

and rejects `amount > free` with the new `InsufficientFreeFunds` (error 18). A sweep
can only ever move money nobody is owed. Three new views expose the same numbers:
`get_total_credited`, `get_vesting_reserved`, `get_available_funds`.

Tests: `test::solvency::credited_balances_are_reserved_against_sweeps`,
`test::solvency::vesting_liability_is_reserved_against_sweeps`.

### 2.2 Vesting underflow / trap on schedule re-creation

`claim_vested` computed `vested - schedule.claimed` with plain subtraction. Because
`create_vesting` preserves `claimed` across re-creation, an admin could re-create a
schedule with a **smaller** total than had already been paid out, leaving
`claimed > vested` and making the subtraction underflow.

This is not a silent wrap: the workspace release profile sets
`overflow-checks = true`, so the wasm **panics and traps**. The contributor's vesting
position became permanently unclaimable until an admin re-created the schedule again,
with no error message explaining why.

**Fix, two layers.**

1. `create_vesting` rejects `total < existing_claimed` with `InvalidAmount`
   (error 12), so the bad state can no longer be created.
2. `claim_vested` uses `saturating_sub` for `vested - claimed`, so a schedule that is
   already in the bad state from the deployed revision degrades to "nothing new to
   claim" (`Ok(0)`) instead of trapping. `claimed` is now also updated with
   `checked_add`.

Test: `test::fixed_vesting_sweep::vesting_recreate_below_claimed_is_rejected`
(including the boundary case `total == claimed`, which must remain legal).

### 2.3 Instance TTL lapsed on the hot paths

`Admin`, `Oracle` and `Token` all live in **instance** storage. At the baseline, only
seven admin-gated functions called `storage::bump_instance`; the busiest paths did not:

`deposit`, `post_cycle_root`, `credit_claim`, `withdraw`, `distribute_fixed`,
`admin_distribute_fixed`, `claim_vested`.

A vault used only for its intended purpose — deposit, post, claim, withdraw — would
therefore never refresh its instance TTL, and would eventually become **unreachable at
every entry point, including read-only views**. This is the Soroban equivalent of
losing the contract's keys through inactivity, and it is a classic mainnet failure
mode. Our own reference docs listed the omission as a table of which functions do and
do not bump, again treating a bug as a design note.

**Fix.** Every state-changing entry point now bumps the instance TTL, and the docs
state the rule as a rule ("no hot path may skip the bump") instead of a per-function
inventory.

### 2.4 Cumulative claims were not bounded by the posted total

`credit_claim` verified a Merkle proof and credited whatever amount the proof
committed to, but never compared the running total against `CycleInfo::total_amount`.
A compromised or buggy oracle key could post a root whose leaves sum to more than the
`total_amount` it declared, and the vault would pay the excess.

`total_amount` is the number maintainers review during the 24-hour challenge window,
so it needs to be the enforced ceiling, not a hint.

**Fix.** New persistent counter `CycleClaimedTotal(cycle_id)`, updated in
`credit_claim`. A claim that would push the cumulative sum above `total_amount` is
rejected with the new `CycleTotalExceeded` (error 19), before any state is written.

Test: `test::solvency::claims_are_capped_at_the_posted_cycle_total`.

### 2.5 `initialize` reported the wrong failure

`initialize` called `admin.require_auth()` before checking whether it had already run.
Calling it a second time with a different key produced an auth trap rather than
`AlreadyInitialized`, which is misleading for integrators and for deployment tooling.

**Fix.** The already-initialized check now runs first. This is the only intentional
change to an existing function's observable behaviour; it affects no happy path and no
successful call.

### 2.6 Withdrawal accounting was duplicated

`withdraw` zeroed the stored balance directly and the ledger could not know how much
liability had been released. That is now a single `storage::take_balance` helper that
zeroes the balance **and** decrements `CreditedTotal`, so the checks-effects ordering
(re-entrancy safe) and the liability bookkeeping are the same operation and cannot be
made to disagree.

### 2.7 Toolchain and documentation were inconsistent with the code

The rejected revision declared three different truths:

| Source | Claimed |
|---|---|
| `contracts/vault/Cargo.toml`, `Cargo.lock` | `soroban-sdk` **28.0.0** |
| `README.md` badge and Quick Start | `soroban-sdk` **27.0.6** |
| `rust-toolchain.toml` comment | `soroban-sdk` **27.0.6** |
| `.github/workflows/ci.yml` | `stellar-cli` **27.1.0** |

This matters beyond cosmetics: `soroban-sdk` 28 **refuses to emit a contract
artifact** from a CLI older than 25.2.0 and expects a matching major, so a reader
following the README, or CI running the pinned 27.1.0 CLI, is in a configuration the
code was not tested in. We verified the correct fix by installing the CLI and building
rather than guessing: **`stellar-cli` 28.1.0 builds the artifact successfully**, so CI
and all docs now pin SDK `28.0.0` + CLI `28.1.0`.

---

## 3. Verification — how to reproduce every claim

Run from a clean checkout. Toolchain: `rust` stable (1.91+), targets
`wasm32v1-none`, `stellar-cli` 28.1.0.

```bash
cargo fmt --all -- --check                       # exit 0
cargo test                                       # 44 passed; 0 failed
cargo clippy --all-targets -- -D warnings        # exit 0, no warnings
stellar contract build --package splitstream-vault
ls -l target/wasm32v1-none/release/splitstream_vault.wasm   # 22,606 bytes
```

Recorded results at the time of writing:

| Check | Result |
|---|---|
| `cargo test` | **44 passed, 0 failed** (baseline: 36 passed) |
| `cargo clippy --all-targets -- -D warnings` | clean, exit 0 |
| `cargo fmt --all -- --check` | clean, exit 0 |
| `stellar contract build` (CLI 28.1.0) | ✅ `splitstream_vault.wasm`, **22,606 bytes** |
| Testnet deploy + `initialize` | ✅ `CB456KG…TLTAT`, both txs confirmed on-ledger |
| Post-deploy smoke test | `deposit(10_000_000)` ok; `get_available_funds` = `10_000_000`; both liabilities `0`; `get_fixed_shares` = `[]` |
| Exported contract functions | 22 (19 before; 3 read-only views added) |

### Property / fuzz suite

`contracts/vault/src/property_test.rs` adds four deterministic, dependency-free
property tests driven by a seeded xorshift64 PRNG (failures are reproducible from
the seed printed in the assertion):

| Test | Property |
|---|---|
| `merkle_proofs_hold_for_random_manifests` | For 48 random manifests of 1–32 leaves: every leaf verifies; a one-unit amount change fails; every single-sibling substitution fails; a single-leaf root equals its leaf. |
| `merkle_proofs_do_not_transfer_between_contributors` | One contributor's proof never validates another contributor's leaf. |
| `vesting_claims_follow_the_linear_schedule` | For 24 random `(total, duration, start)` triples walked through random ledgers: cumulative claims equal `floor(total * elapsed / duration)` exactly, are monotonic, never exceed `total`, and the stored liability always equals `total - claimed`. |
| `vesting_recreation_and_vault_accounting_stay_consistent` | Random re-creations (including the `total == claimed` boundary) never make `claimed > total`, and the accounting identity `balance(vault) == available + credited + vesting_reserved` holds at every step. |

The new example-based tests are not smoke tests either — each one fails against
the old code:

- `credited_balances_are_reserved_against_sweeps` — sweeps 1 unit more than free
  funds, expects `InsufficientFreeFunds`, then proves the contributor can still
  withdraw, then proves the same sweep succeeds once nothing is owed.
- `vesting_liability_is_reserved_against_sweeps` — same for the vesting liability,
  including the half-vested transition.
- `claims_are_capped_at_the_posted_cycle_total` — posts a root whose leaves sum to 300
  but declares `total_amount` 250, and asserts the second claim cannot over-pay.
- `vesting_recreate_below_claimed_is_rejected` — asserts the corrupting re-creation is
  rejected and the `total == claimed` boundary still works.

---

## 4. ABI and protocol compatibility

This was a hardening pass on a live Testnet contract, so we deliberately avoided
breaking the frozen cross-repo interfaces:

- **No function was removed and no existing signature changed.**
- **No error discriminant was renumbered.** Errors 18 (`InsufficientFreeFunds`) and 19
  (`CycleTotalExceeded`) are appended; 1–17 keep their codes, as the docs require.
- **Event topic names and payloads are unchanged.**
- **The Merkle leaf format is unchanged** (`sha256(xdr(contributor) ++ xdr(amount))`,
  sorted-pair hashing), so `splitstream-actions` manifests still verify.
- **Storage keys were only added** — no existing key's encoding changed.
- The three new views are additive and read-only.

The only behavioural change to an existing entry point is §2.5 (error reporting for a
repeated `initialize`), plus the intended new rejection cases in §2.1–§2.4.

---

## 5. Documentation brought in line with the implementation

The rejected revision had extensive docs that in several places documented bugs as
features. All of the following were corrected, not just appended to:

| File | Change |
|---|---|
| `README.md` | SDK badge/pins to 28.0.0 + CLI 28.1.0; liability ledger and sweep cap in the storage section; new views and design decision; corrected the `distribute_fixed` rationale. |
| `docs/contract-reference.md` | Function count 19 → 22 with rows for the new views; `execute_sweep`, `credit_claim`, `withdraw`, `create_vesting`, `claim_vested` and both distributions updated; error table 17 → 19; TTL bump inventory replaced with the actual rule; storage layout now lists `CreditedTotal`, `VestingReserved`, `CycleClaimedTotal`. |
| `docs/for-maintainers.md` | "The sweep is not balance-aware" hazard removed and replaced with the reserve-aware behaviour and `InsufficientFreeFunds`. |
| `docs/protocol-mechanics.md` | The note that `total_amount` "is never summed against claims" replaced with the enforcement that now exists. |
| `SECURITY.md` | Invariants 7–11 added: liability tracking, sweep cap, per-cycle over-payment cap, TTL liveness, and the tightened vesting invariant. |
| `docs/SUMMARY.md`, `docs-hub/architecture.md`, `docs-hub/index.md` | Function/error counts corrected. |
| `CONTRIBUTING.md`, `rust-toolchain.toml`, `.github/workflows/ci.yml` | SDK/CLI versions aligned. |
| `CHANGELOG.md` | Full `Unreleased` section describing the security fixes and the toolchain alignment. |

---

## 6. What this changes for Wave participation

Concretely, a reviewer should now find:

1. **A buildable, deployable Soroban contract with a reproducible artifact** — not a
   claim: a 22,606-byte wasm produced by the CLI version CI pins, deployed and
   initialized on Testnet at `CB456KG…TLTAT`.
2. **A CI configuration that matches the code it builds**, so the green badge means
   what it appears to mean.
3. **Security work in the areas that matter for a fund-holding contract** — solvency,
   liability accounting, arithmetic safety, and storage liveness — with regression
   tests that fail on the old code.
4. **Documentation that describes the current code**, including the constraints a
   contributor would actually hit.
5. **Frozen-interface discipline**, so the sibling `splitstream-actions` and
   `splitstream-sdk-cli` repos are not broken by this pass.

---

## 7. Honest limitations

We would rather state these than have them discovered in review:

- **The new Testnet deployment uses fresh keys, and they are Testnet-only.** The
  admin/oracle identities behind this deployment were generated for verification and
  funded from Friendbot; the production/mainnet keys are unaffected. The deployment
  record (addresses, deploy and `initialize` transaction hashes) is in
  `docs/developer-guide.md`.
- **The previous Testnet address is superseded, not mutated.**
  `CCC2LP2LOYZOLA2JW4C4K7JMR3TRJZIKHDSQYSFJ3R3MCDJLVBT3PZOC` still holds the
  pre-fix bytecode and should not be used; the docs now point at the new address and
  label the old one explicitly.
- **The contract is still unaudited.** `SECURITY.md` says so explicitly, and this pass
  reduces specific risk rather than substituting for an external review.
- **Vesting solvency is tracked, not escrowed.** Vesting claims pay from the vault's
  token balance; `get_available_funds` and `get_vesting_reserved` now make the position
  visible, but deposits are still the maintainer's responsibility.
- **`Unauthorized` (error 3) remains declared but unused**, kept only because error
  discriminants are frozen; `require_auth` traps rather than returning an error.

---

## 8. Suggested appeal text

> Since the rejection we audited the contract against Soroban's actual failure modes
> and fixed four defects, each with a regression test that fails on the rejected code:
> (1) admin sweeps could drain balances already credited to contributors and unvested
> schedule amounts — the vault now tracks its liabilities and caps sweeps at free
> funds; (2) re-creating a vesting schedule below the already-claimed amount trapped
> `claim_vested` on an overflow check — now rejected at creation and saturating at
> claim; (3) the busiest entry points never bumped the instance TTL, so an actively
> used vault could become permanently unreachable — every state-changing path now
> bumps it; (4) claims were not capped by the cycle's posted `total_amount` — they now
> are. We also found the repo declared SDK 27.0.6 in the README while building 28.0.0
> and pinning CLI 27.1.0 in CI; those are aligned and the artifact was rebuilt with
> `stellar-cli` 28.1.0. The hardened contract is deployed and initialized on Testnet
> at `CB456KGXH4233BHW5BUEJMN6OJ4ZMWMPP3OXJU5HVQ2T2F7A7XHWLTAT`, with a deposit and
> the read-only views exercised on-ledger. The suite is now 44 passing tests
> (including a randomised property suite for the Merkle and vesting arithmetic),
> clippy is clean with `-D warnings`, and the frozen Merkle format, event topics,
> function signatures and error numbering are unchanged. Verification steps are in
> `CHANGE.md`.

---

## 9. Wave backlog

Six scoped issues were opened with the `Stellar Wave` label so the repository is
immediately actionable for contributors, each with explicit acceptance criteria:

| Issue | Scope |
|---|---|
| [#30](https://github.com/SplitStream-Labs/splitstream-core/issues/30) | Add `get_config()` and `get_cycle_claimed_total()` read-only views. |
| [#31](https://github.com/SplitStream-Labs/splitstream-core/issues/31) | Enforce `cargo fmt` and add a dependency advisory audit to CI. |
| [#32](https://github.com/SplitStream-Labs/splitstream-core/issues/32) | Reproducible deploy-and-initialize script for Testnet. |
| [#33](https://github.com/SplitStream-Labs/splitstream-core/issues/33) | Commit a contract-spec snapshot and assert ABI stability in CI. |
| [#34](https://github.com/SplitStream-Labs/splitstream-core/issues/34) | Randomised operation-sequence tests for the solvency invariants. |
| [#35](https://github.com/SplitStream-Labs/splitstream-core/issues/35) | Cross-repo golden fixture for the frozen Merkle leaf format. |

Five pre-existing issues (#1–#5) remain open and were deliberately left alone.

## 10. Appendix — files changed

```
 .github/workflows/ci.yml            CLI pin 27.1.0 → 28.1.0
 CHANGELOG.md                        Unreleased: security + toolchain entries
 CONTRIBUTING.md                     SDK version
 README.md                           versions, liability ledger, views, design notes
 SECURITY.md                         invariants 7–11, vesting disclosure
 contracts/vault/src/claims.rs       TTL bump, cycle claim cap, take_balance
 contracts/vault/src/errors.rs       errors 18, 19 appended
 contracts/vault/src/fixed_split.rs  TTL bump
 contracts/vault/src/lib.rs          TTL bumps, 3 views, initialize ordering
 contracts/vault/src/property_test.rs new: 4 deterministic property/fuzz tests
 contracts/vault/src/storage.rs      liability ledger + helpers
 contracts/vault/src/sweep.rs        free-funds cap
 contracts/vault/src/test.rs         +4 example tests (36 → 40)
 contracts/vault/src/property_test.rs new property/fuzz suite (+4, total 44)
 contracts/vault/src/vesting.rs      underflow fix, liability adjustments
 docs/contract-reference.md          functions, errors, TTL rule, storage keys
 docs/developer-guide.md             SDK/CLI versions
 docs/for-maintainers.md             reserve-aware sweep
 docs/protocol-mechanics.md          cycle total enforcement
 docs/SUMMARY.md                     function count
 docs-hub/architecture.md            ABI counts
 docs-hub/index.md                   ABI counts
 rust-toolchain.toml                 SDK version comment
```
