# Security Policy

## Reporting a vulnerability

If you find a security issue in SplitStreamVault — a way to claim funds you are
not entitled to, to bypass an auth check, to grief the dispute window, or to
otherwise move funds unexpectedly — **do not open a public issue**. Report it
privately to the repository maintainers (GitHub security advisories or a direct
maintainer message) and include:

- the affected function(s) and the exact call sequence to reproduce,
- why it matters (impact),
- a suggested fix, if you have one.

We aim to acknowledge reports within 48 hours and to ship a fix (and, where
relevant, a migration path) before any public disclosure.

## Audit status

SplitStreamVault has **not** undergone an independent third-party security
audit. The threat model and invariants below reflect our own analysis and
test coverage, not an external review. Treat this as pre-audit software:
appropriate for Testnet and for controlled Mainnet use with amounts you can
afford to lose, not yet for large unattended treasuries.

## Threat model

The vault holds pooled funds in a single SEP-41 token. Actors and trust
boundaries:

- **Admin** — governance: initializes the vault, replaces cycle roots within
  the dispute window, configures fixed shares, creates vesting schedules, and
  requests/executes/cancels sweeps. A compromised admin key cannot instantly
  drain the pool: sweeps are subject to a 72-hour timelock, and the admin
  cannot override the oracle's dispute window (root replacement is admin-only
  but bounded by the 24h window and one-replacement rule).
- **Oracle** — the CI relay: posts cycle roots and triggers fixed
  distributions. It cannot withdraw funds itself; a malicious root is
  challengeable by the admin for 24h before any claim can extract funds.
- **Contributors** — can only claim what they can prove against the stored
  root, once per cycle, and can only withdraw their own credited balance.

### Key invariants

1. **A posted root is not claimable until 24h have elapsed** since
   `posted_at` — the dispute window is enforced in `credit_claim`, not just
   advised by the UI.
2. **A root never changes after real funds have moved against it** —
   `challenge_and_replace_root` is unreachable once `claims_started` is true.
3. **The dispute window never restarts** — replacement keeps the original
   `posted_at`, bounding total dispute time to 24h per cycle.
4. **No double claims** — `CycleClaimed(cycle_id, contributor)`.
5. **Withdraw zeroes the balance before transferring** (checks-effects-
   interactions), preventing re-entrant double-withdrawals.
6. **Re-created vesting schedules preserve `claimed`** — no re-claiming
   already-vested funds, and a schedule can never be re-created below the
   amount already paid out.
7. **Liabilities are tracked, not inferred.** `CreditedTotal` and
   `VestingReserved` in instance storage mirror what the vault owes; every
   balance write adjusts them in the same transaction.
8. **A sweep can never strand a claim.** `execute_sweep` is bounded by
   `balance(vault) - CreditedTotal - VestingReserved`, so the admin cannot
   move funds that back credited balances or unvested schedule remaining.
9. **No over-payment per cycle.** Cumulative claims per cycle cannot exceed the
   `total_amount` the oracle committed to, even if the committed root's leaves
   sum higher.
10. **All math is integer basis-points** — no float rounding drift, and no
   unbounded loops over contributor sets.
11. **No entry point can be bricked by a lapsed instance TTL** — every
   state-changing function bumps the instance TTL, since `Admin`, `Oracle` and
   `Token` live there and a lapsed instance is unreachable.

## Known considerations

- `distribute_fixed` is oracle-gated only (the SDK has no caller-introspection
  API); see the README design-decisions section. The oracle is an operational
  trust anchor — its key should be held by the CI system with appropriate
  controls.
- Vesting rewards are paid directly from the vault's token balance rather than
  credited to a pull-payment balance, so a vesting claim relies on the vault
  holding the funds. `get_available_funds` and `get_vesting_reserved` expose the
  position; fund deposits before creating schedules.
- Basis-point splits truncate toward zero per recipient, so a distribution can
  leave dust in the vault; it is swept or re-distributed in later cycles.
- The challenge window and sweep timelock are measured in ledger **timestamps**
  (seconds), not ledger sequence numbers, per the spec.