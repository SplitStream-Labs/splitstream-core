# For maintainers

The admin-facing operations that live in this contract. All of them are
admin-gated except `distribute_fixed`, which is oracle-gated.

The admin and oracle addresses are set once in `initialize` and cannot be
changed: this contract has no rotation function for either key, and no function
to change the settlement token. Treat both keys as permanent for a deployment.

## Configuring fixed-share waterfalls

```rust
configure_fixed_shares(shares: Vec<(Address, u32)>)   // admin auth
```

A real, valid split:

```rust
[
  (team_address,    5_000),  // 50.00%
  (community,       3_000),  // 30.00%
  (reserve,         2_000),  // 20.00%
]                            //           = 10_000
```

Rules and gotchas:

- The basis points must sum to **exactly** 10 000, or the call fails with
  `InvalidShareTotal` (9). 9 999 is as invalid as 5 001.
- An empty list sums to 0 and is therefore also rejected. You cannot clear the
  split list once configured — you can only replace it with another list that
  sums to 10 000.
- The call overwrites the previous list in one write. Re-configuring is safe and
  emits `fixed_shares_configured` with the new list.
- `get_fixed_shares()` reads the current list. It returns an empty vector when
  the list was never configured.

Distributing:

| Function | Auth | When to use |
|---|---|---|
| `distribute_fixed(amount)` | oracle | The automated cycle trigger. |
| `admin_distribute_fixed(amount)` | admin | Manual trigger when no oracle key is available. |

Both share one internal body, so they credit identical amounts and emit
identical `fixed_distributed` events. Both reject a non-positive `amount` with
`InvalidAmount`, and both are a no-op when the share list is empty.

Amounts are integer math with truncation per recipient: `amount * bps / 10_000`.
Distributing 9 across 50/30/20 credits 4, 2 and 1 — 7 of the 9 — and the
remaining 2 stay in the vault as reclaimable dust. Distribute amounts large
enough that this is negligible.

Distribution credits pull-payment balances; it does not move tokens. Keep the
vault funded with `deposit` so recipients can `withdraw`.

## Creating vesting schedules

```rust
create_vesting(contributor: Address, total: i128, duration_ledgers: u32)   // admin auth
```

A real schedule, and what it pays:

| Ledger | Elapsed | Vested | `claim_vested` returns |
|---|---|---|---|
| 100 (`start_ledger`) | 0 / 100 | 0 | 0 |
| 150 | 50 / 100 | 500 | 500 |
| 150 (again) | 50 / 100 | 500 | 0 |
| 200 | 100 / 100 | 1 000 | 500 |
| 200 (again) | 100 / 100 | 1 000 | 0 |

That is `total = 1_000`, `duration_ledgers = 100`, created at ledger 100.
Vesting is linear in ledger sequence, and elapsed time is capped at the
duration, so a schedule never vests more than `total`.

Notes:

- `vested` is a global figure, not an increment: `claim_vested` transfers
  `vested - claimed` and records the new `claimed`. A second claim in the same
  ledger returns 0, which is a legitimate state, not an error.
- `claim_vested` transfers directly to the contributor — it does not credit a
  pull-payment balance. Vesting is a separate accounting track from
  `get_balance`.
- Re-creating a schedule **preserves `claimed`**. That is the safety property:
  replacing a schedule can never let a contributor re-claim what they already
  took. Only `total`, `start_ledger` and `duration_ledgers` change, and
  `start_ledger` resets to the current ledger — so a re-created schedule vests
  from scratch while still remembering past claims.
- `total <= 0` or `duration_ledgers == 0` fails with `InvalidAmount`.
- A contributor with no schedule gets `NoVestingSchedule` (17) from
  `claim_vested`, distinct from a zero balance.
- The vault must hold the tokens at claim time. Fund it with `deposit`.

## Disputing a cycle

```rust
challenge_and_replace_root(cycle_id, new_root, new_total_amount)   // admin auth
```

Use it when, after the oracle posted a cycle root, you find a real error in the
points that root encodes — a merged PR missing, a duplicated address, a wrong
weight — and **no one has claimed yet**.

The constraint is a hard 24 hours from the original post (`posted_at`), and one
replacement per cycle:

| Situation | Result |
|---|---|
| Within 24h, no claims yet | Root replaced. `replaced = true`. |
| Second replacement | `RootAlreadyReplaced` (15) |
| 24h elapsed (`elapsed >= 86_400`) | `ChallengeWindowClosed` (14) |
| Any claim has succeeded | `ClaimsAlreadyStarted` (16) — checked first, even after the window closes |
| Unknown `cycle_id` | `CycleNotFound` (5) |

`posted_at` is preserved across a replacement, so the window never restarts.
A replacement at hour 23 leaves you one hour; a replacement at hour 23 of an
already-replaced cycle is rejected outright.

After a replacement the new root governs. Proofs built for the old manifest fail
with `InvalidProof`, so re-publishing the corrected manifest off-chain is part
of the job, not an afterthought.

**If you miss the window,** the root is final. No admin call can change it:
`challenge_and_replace_root` returns `ChallengeWindowClosed`, and
`post_cycle_root` returns `CycleAlreadyPosted` for that cycle. There is no
cancel or re-post per cycle. The corrected distribution has to run as a new
cycle with a new root and its own 24-hour window. The only other lever over
pooled funds is a sweep — which is a blunt, whole-vault instrument, not a
correction.

## The sweep mechanism

The emergency exit for pooled funds, protected by a 72-hour timelock so a
compromised admin key cannot drain the vault in one transaction.

```rust
request_sweep(to: Address, amount: i128)   // admin auth — starts the 72h clock
execute_sweep()                            // admin auth — after 72h
cancel_sweep()                             // admin auth — any time
```

| Step | Behavior |
|---|---|
| `request_sweep` | Registers `{ to, amount, requested_at }`. Rejects `amount <= 0` with `InvalidAmount`. Emits `sweep_requested`. |
| `request_sweep` again | Overwrites the pending request and **restarts the timelock** from the new `requested_at`. |
| `execute_sweep` | Requires `elapsed >= SWEEP_TIMELOCK_SECS = 259_200` (72h) and `amount <=` the vault's free funds. Transfers the amount, clears the request, emits `sweep_executed`. |
| `execute_sweep` before 72h | `SweepNotReady` (10). |
| `execute_sweep` above free funds | `InsufficientFreeFunds` (18). Free funds are the token balance minus `get_total_credited()` and `get_vesting_reserved()` — see `get_available_funds()`. |
| `execute_sweep` with no request | `NoSweepPending` (11). |
| `cancel_sweep` | Clears any pending request, idempotent, emits `sweep_cancelled` only if a request existed. |

The timelock is measured in ledger timestamp seconds, not sequence numbers.

Two things to know before sweeping:

- **The sweep is reserve-aware.** It can only move funds above every
  outstanding `Balance` entry and the outstanding vesting liability. If you
  request more than `get_available_funds()` returns, `execute_sweep` fails with
  `InsufficientFreeFunds` — contributors' `withdraw` and `claim_vested` calls
  can no longer be stranded by a sweep. To release pool funds, distribute or
  settle the liabilities first, then sweep what is left.
- **Only the admin can cancel.** `cancel_sweep` requires the same key that
  calls `execute_sweep`, so the timelock is not a veto anyone else holds. What
  it buys is 72 hours of visibility: contributors can `withdraw` credited
  balances and `claim_vested` accrued amounts before the transfer lands.
  Overwriting a request restarts the clock, which extends that window again.
