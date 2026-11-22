# Protocol mechanics

## The cycle lifecycle

A cycle runs through six steps. Steps 3, 5 and 6 happen in this contract; the
rest happen elsewhere.

1. **Contributors open and merge PRs.** *(elsewhere — the project's code repo.)*
2. **The manifest is built.** *(elsewhere — splitstream-actions scores the
   merged PRs and computes the Merkle root over the `(contributor, amount)`
   leaves.)*
3. **`post_cycle_root(cycle_id, root, total_amount)`** — the oracle posts the
   root. Oracle auth. One post per cycle. The cycle enters the challenge window.
4. **The 24-hour challenge window runs.** `CHALLENGE_WINDOW_SECS = 86_400`
   seconds from `posted_at`. Claims are rejected until it elapses; the admin may
   replace the root once (see below).
5. **`credit_claim(contributor, cycle_id, amount, proof)`** — the contributor
   proves their `(address, amount)` against the stored root. Contributor auth.
   On success the amount is credited to a pull-payment balance and the root is
   locked forever.
6. **`withdraw(contributor)`** — the contributor's full balance is transferred
   to them. Contributor auth.

The contract is pull-payment throughout. Steps 5 and 6 are separate calls: a
claim never pushes tokens to a contributor, and the contract never iterates a
contributor list.

## Worked example: a Merkle claim

The leaf format is frozen:

```
leaf = sha256(contributor.to_xdr(env) ++ amount.to_xdr(env))
```

Siblings are hashed in sorted-pair order, so proof order never matters. Both
conventions are computed in `merkle.rs` and must match splitstream-actions
byte-for-byte.

A manifest for cycle `1` with two contributors:

| Contributor | Amount |
|---|---|
| A | 400 |
| B | 600 |
| | **1 000 total** |

1. The oracle calls `post_cycle_root(1, root, 1_000)` at timestamp `t`:
   `CycleInfo { root, total_amount: 1_000, posted_at: t, claims_started: false,
   replaced: false }`.
2. At `t + 86_399` — one second short of the window — A calls
   `credit_claim(A, 1, 400, proof)`. Rejected with `ClaimsNotYetOpen`. The window
   is inclusive: claims open at exactly `t + 86_400`.
3. At `t + 86_400`, A claims 400 with proof `[leaf_hash(B, 600)]`. A's leaf
   hashes with that sibling to the stored root, so A is credited 400 and
   `claims_started` is set to `true` for cycle `1`.
4. B claims 600 with proof `[leaf_hash(A, 400)]`. Credited 600.
5. A calls `withdraw(A)`: balance 400 is zeroed, then 400 tokens are
   transferred. B calls `withdraw(B)`: 600 tokens. Total outflow 1 000, equal to
   `total_amount`.

Note that this contract verifies the proof and pays. It does not compute the
points behind 400 and 600 — that arithmetic lives in splitstream-actions. The
contract does, however, track the cumulative amount claimed per cycle and reject
any claim that would push it above the posted `total_amount`
(`CycleTotalExceeded`, 19), so a root whose leaves sum higher than the total it
was posted with cannot over-pay.

## Worked example: fixed splits and dust

The fixed-split formula is integer basis-point math with truncation toward zero,
per recipient:

```
share = amount * bps / 10_000
```

With shares of `5_000` / `3_000` / `2_000` bps:

| Call | 50.00% | 30.00% | 20.00% | Credited | Left in vault |
|---|---|---|---|---|---|
| `distribute_fixed(1_000)` | 500 | 300 | 200 | 1 000 | 0 |
| `distribute_fixed(9)` | 4 | 2 | 1 | 7 | 2 |

The second row shows dust. `9 * 5_000 / 10_000 = 4.5` truncates to 4,
`9 * 3_000 / 10_000 = 2.7` to 2, and `9 * 2_000 / 10_000 = 1.8` to 1. Seven
units are credited; the remaining 2 stay in the vault. After both calls the
balances are 504 / 302 / 201.

Dust is never redistributed. Split amounts large enough to distribute with
negligible truncation, or accept that the remainder accumulates in the vault.

## The challenge window

A posted root is **not** immediately claimable. It is provisional for 24 hours.

**What a dispute looks like.** The oracle posts a root, and maintainers see
something wrong in the cycle it encodes: a merged PR missing, a duplicate
address, a weight that does not match the agreed scoring. No contributor can
have claimed yet, because claims are still closed.

**Who can act.** The admin, and only the admin, via:

```rust
challenge_and_replace_root(cycle_id, new_root, new_total_amount)
```

This is deliberately not oracle-gated. The automated relay posts roots; it cannot
override its own dispute window.

**The checks, in order.** `ClaimsAlreadyStarted` if any claim has succeeded;
`ChallengeWindowClosed` if 24 hours have elapsed since `posted_at`;
`RootAlreadyReplaced` on a second replacement; `CycleNotFound` for an unknown
cycle. Exactly one replacement is allowed.

**The window does not restart.** `posted_at` is preserved through a replacement,
so total dispute time is bounded to 24 hours from the original post. A
replacement at hour 23 leaves one hour.

**After a replacement.** The stored root is the new one, so every claim must
prove against the new manifest. Proofs built for the old root fail with
`InvalidProof`.

**The permanent lock.** The first successful `credit_claim` sets
`claims_started = true`. From that moment `challenge_and_replace_root` is
rejected with `ClaimsAlreadyStarted` — permanently, even if the 24 hours have
not elapsed. One contributor claiming is enough to finalize the root for
everyone.

**The boundary.** The two paths cross cleanly at exactly `t + 86_400`:
replacement requires `elapsed < 86_400`, and claims require
`elapsed >= 86_400`. At that instant replacement is closed and claims are open.

**If you miss the window.** The root is final and no admin call can change it.
There is no per-cycle cancel or re-post — `post_cycle_root` rejects an
already-posted cycle with `CycleAlreadyPosted`. A corrected distribution has to
be posted as a new cycle root, subject to a new 24-hour window. The only other
admin lever over pooled funds is the sweep (72-hour timelock, below).

## The three settlement strategies

**Merkle-proof claims** — Cycle-based points payouts. The oracle posts a
manifest root with `post_cycle_root` (oracle auth). After the challenge window,
contributors call `credit_claim` (contributor auth) to prove a
`(contributor, amount)` leaf against the stored root and are credited a
pull-payment balance, which they then pull with `withdraw` (contributor auth).
The root is locked by the first successful claim.

**Fixed basis-point waterfalls** — recurring team/community/reserve splits. The
admin calls `configure_fixed_shares` (admin auth) with a share list whose basis
points sum to exactly 10 000, or it is rejected with `InvalidShareTotal`.
Distributions are triggered by the oracle via `distribute_fixed` (oracle auth)
or manually by the admin via `admin_distribute_fixed` (admin auth). Both credit
each recipient `amount * bps / 10_000` to their pull-payment balance and emit
one event per recipient; they share one internal body and cannot drift. An
empty share list makes a distribution a no-op.

**Linear vesting streams** — maintainer retention rewards. The admin calls
`create_vesting(contributor, total, duration_ledgers)` (admin auth) to create or
re-create a schedule. The contributor calls `claim_vested` (contributor auth) to
claim whatever has newly vested, which is transferred directly rather than
credited to a pull-payment balance. Re-creating a schedule preserves the
already-claimed amount, so a replaced schedule can never pay twice.
