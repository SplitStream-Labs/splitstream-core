# Contract reference

Everything below is read from `contracts/vault/src/` at the current revision.
Numeric error discriminants are part of the on-chain ABI and must not be
renumbered.

## Public functions

22 public functions: 14 state-changing, 8 read-only views. Events are emitted
through `Events::publish` with a topic tuple and a data payload; the payload is
`()` unless noted.

| Function | Auth | Parameters | Behavior | Event |
|---|---|---|---|---|
| `initialize` | admin | `admin: Address, oracle: Address, token: Address` | One-time setup. Stores admin, oracle and the SEP-41 token in instance storage; bumps instance TTL. | `initialized(admin, oracle, token)` |
| `deposit` | `from` | `from: Address, amount: i128` | Pulls `amount` of the token from `from` into the vault. Rejects `amount <= 0` with `InvalidAmount`. | `deposit(from, amount)` |
| `post_cycle_root` | oracle | `cycle_id: u64, root: BytesN<32>, total_amount: i128` | Posts a cycle's manifest root. One post per cycle; rejects a repeat with `CycleAlreadyPosted`. Starts the challenge window at the current ledger timestamp. | `cycle_posted(cycle_id, root, total_amount)` |
| `challenge_and_replace_root` | admin | `cycle_id: u64, new_root: BytesN<32>, new_total_amount: i128` | Replaces a cycle root once, inside the 24h window. Preserves `posted_at`. See [protocol mechanics](protocol-mechanics.md#the-challenge-window). | `cycle_root_replaced(cycle_id, new_root, new_total_amount)` |
| `credit_claim` | contributor | `contributor: Address, cycle_id: u64, amount: i128, proof: Vec<BytesN<32>>` | Verifies the Merkle proof against the current root and credits the pull-payment balance. Rejects claims before the window closes, double claims, bad proofs, and cumulative claims above the cycle's `total_amount` (`CycleTotalExceeded`). Sets `claims_started`, locking the root. Bumps instance TTL. | `claim_credited(contributor, cycle_id, amount)` |
| `withdraw` | contributor | `contributor: Address` | Pays out the full claimable balance. Zeroes the stored balance and drops the `CreditedTotal` liability before the token transfer (checks-effects-interactions). Rejects an empty balance with `InsufficientBalance`. Bumps instance TTL. | `withdrawn(contributor, balance)` |
| `configure_fixed_shares` | admin | `shares: Vec<(Address, u32)>` | Validates and stores the basis-point share list. Rejects a sum other than 10 000 with `InvalidShareTotal`. Bumps instance TTL. | `fixed_shares_configured(shares)`; data payload is the share list, topic is the bare symbol |
| `distribute_fixed` | oracle | `amount: i128` | Credits each recipient `amount * bps / 10_000`. Rejects `amount <= 0` with `InvalidAmount`. Empty share list is a no-op. Bumps instance TTL. | `fixed_distributed(address, share)` once per recipient |
| `admin_distribute_fixed` | admin | `amount: i128` | Same body, event and errors as `distribute_fixed`; only the authorization differs. Bumps instance TTL. | `fixed_distributed(address, share)` once per recipient |
| `create_vesting` | admin | `contributor: Address, total: i128, duration_ledgers: u32` | Creates or re-creates a linear schedule, preserving any already-claimed amount. Rejects `total <= 0`, `duration_ledgers == 0`, or a `total` below the already-claimed amount with `InvalidAmount`. Adjusts `VestingReserved` by the change in outstanding liability. Bumps instance TTL. | `vesting_created(contributor, total, duration_ledgers)` |
| `claim_vested` | contributor | `contributor: Address` → `i128` | Transfers the newly vested amount directly, decreasing `VestingReserved` by the same amount. Returns the amount claimed; `0` means nothing new has vested and is not an error. Bumps instance TTL. | `vested_claimed(contributor, claimable)`; none when the amount is 0 |
| `request_sweep` | admin | `to: Address, amount: i128` | Registers or overwrites a sweep request, starting the 72h timelock. Rejects `amount <= 0` with `InvalidAmount`. Bumps instance TTL. | `sweep_requested(to, amount)` |
| `execute_sweep` | admin | — | Transfers the requested amount to the destination and clears the request. Rejects an early call with `SweepNotReady`, and any amount above the vault's free funds (balance minus credited balances and vesting liability) with `InsufficientFreeFunds`. Bumps instance TTL. | `sweep_executed(to, amount)` |
| `cancel_sweep` | admin | — | Clears any pending request. Idempotent: no error and no event when nothing is pending. Bumps instance TTL. | `sweep_cancelled()`; only when a request existed |
| `get_balance` | none | `contributor: Address` → `i128` | Claimable pull-payment balance; `0` when never credited. | — |
| `get_cycle_info` | none | `cycle_id: u64` → `Option<CycleInfo>` | `root`, `total_amount`, `posted_at`, `claims_started`, `replaced`. | — |
| `has_claimed` | none | `cycle_id: u64, contributor: Address` → `bool` | Whether this contributor has claimed for this cycle. | — |
| `get_vesting` | none | `contributor: Address` → `Option<VestingData>` | `total`, `claimed`, `start_ledger`, `duration_ledgers`. | — |
| `get_fixed_shares` | none | — → `Vec<(Address, u32)>` | The configured split list; empty when never configured. | — |
| `get_total_credited` | none | — → `i128` | Sum of every contributor's outstanding pull-payment balance. | — |
| `get_vesting_reserved` | none | — → `i128` | Outstanding vesting liability, `sum(total - claimed)`. | — |
| `get_available_funds` | none | — → `i128` | Token balance minus both liabilities, floored at zero. The cap on `execute_sweep`. | — |

Notes:

- `deposit` and `withdraw` call the SEP-41 token contract's `transfer` through
  `MuxedAddress`.
- `credit_claim` verifies the proof against the root **and** tracks
  `CycleClaimedTotal(cycle_id)`: cumulative claims may not exceed
  `total_amount`, so a root whose leaves sum higher than the posted total cannot
  over-pay.
- Instance TTL is bumped by every state-changing function: `initialize`,
  `deposit`, `post_cycle_root`, `challenge_and_replace_root`, `credit_claim`,
  `withdraw`, `configure_fixed_shares`, `distribute_fixed`,
  `admin_distribute_fixed`, `create_vesting`, `claim_vested`, `request_sweep`,
  `execute_sweep`, `cancel_sweep`. A lapsed instance makes every entry point
  unreachable, including reads, so no hot path may skip the bump.
- `get_available_funds` reads the token contract and needs `Token` to be set;
  before `initialize` it returns `0`.
- `get_total_credited` and `get_vesting_reserved` are the liability ledger the
  sweep cap is derived from. They live in instance storage and are maintained
  transactionally with the balance writes they track.

## Errors

`SplitStreamError` has 19 variants. Variants 1–19 are the complete enum.

| Code | Variant | Meaning |
|---|---|---|
| 1 | `NotInitialized` | The contract has not been initialized. |
| 2 | `AlreadyInitialized` | `initialize` was already called. |
| 3 | `Unauthorized` | The caller is not authorized for this action. |
| 4 | `CycleAlreadyPosted` | A root has already been posted for this cycle. |
| 5 | `CycleNotFound` | No cycle exists for this id. |
| 6 | `AlreadyClaimed` | This contributor has already claimed for this cycle. |
| 7 | `InvalidProof` | The submitted Merkle proof does not match the stored root. |
| 8 | `InsufficientBalance` | No claimable balance to withdraw. |
| 9 | `InvalidShareTotal` | The fixed-share basis points do not sum to 10 000. |
| 10 | `SweepNotReady` | The 72h sweep timelock has not elapsed. |
| 11 | `NoSweepPending` | There is no pending sweep request. |
| 12 | `InvalidAmount` | The amount is not valid for this operation. |
| 13 | `ClaimsNotYetOpen` | The challenge window has not elapsed; claims are not open. |
| 14 | `ChallengeWindowClosed` | The 24h window has closed; the root can no longer be replaced. |
| 15 | `RootAlreadyReplaced` | This cycle's root has already been replaced once. |
| 16 | `ClaimsAlreadyStarted` | Claims have started; the root is final. |
| 17 | `NoVestingSchedule` | The contributor has no vesting schedule to claim against. |
| 18 | `InsufficientFreeFunds` | The sweep would take funds reserved for credited balances or vesting. |
| 19 | `CycleTotalExceeded` | Claims for this cycle would exceed the posted `total_amount`. |

`Unauthorized` (3) is declared but no function returns it: authorization is
enforced by `require_auth`, which aborts the invocation rather than returning an
error.

## Deployed — Testnet

| | |
|---|---|
| Vault contract | `CB456KGXH4233BHW5BUEJMN6OJ4ZMWMPP3OXJU5HVQ2T2F7A7XHWLTAT` |
| Token contract | `CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC` (native XLM SAC) |
| Admin | `GDTEFWNE6KIWYKU4GIVINPIIQ26E2ZMNCHCQOHFUIEFV3E4UJWVROB54` |
| Oracle | `GAW3G4BHBS37J2V3TBX4FGTIY3JXUF6YG44AAQDRV7QE2KHBEMROMJVX` |
| Explorer | https://stellar.expert/explorer/testnet/contract/CB456KGXH4233BHW5BUEJMN6OJ4ZMWMPP3OXJU5HVQ2T2F7A7XHWLTAT |
| Network | Test SDF Network ; September 2015 (Testnet) |
| Deploy tx | https://stellar.expert/explorer/testnet/tx/05486762036ebad10fb488e657fe4efaa625a5a1438b397e86ee09727a937592 |
| `initialize` tx | https://stellar.expert/explorer/testnet/tx/fcf562b961ff3c6c240687eaf6d3546ae1c830d28abc60fa4d08f9af43262483 |

This is the hardened revision. The ABI change is additive: the same 19
functions plus three read-only views (22 total), with no signature or error
renumbering. The earlier deployment at
`CCC2LP2LOYZOLA2JW4C4K7JMR3TRJZIKHDSQYSFJ3R3MCDJLVBT3PZOC` is superseded.

One SEP-41 token per deployment, fixed at `initialize`.

## Storage layout

`DataKey` is the `#[contracttype]` enum addressing every entry:

```rust
pub enum DataKey {
    Admin,                        // instance
    Oracle,                       // instance
    Token,                        // instance
    CycleData(u64),               // persistent
    CycleClaimed(u64, Address),   // persistent
    Balance(Address),             // persistent
    CreditedTotal,                // instance
    VestingReserved,              // instance
    CycleClaimedTotal(u64),       // persistent
    FixedShares,                  // persistent
    Vesting(Address),             // persistent
    SweepRequest,                 // persistent
}
```

**Instance** — `Admin`, `Oracle`, `Token`, `CreditedTotal`, `VestingReserved`.
Small, always needed, and read by nearly every call. Bumped to
`INSTANCE_TTL_LEDGERS = 6_311_520` (~1 year at 5s ledgers) via
`storage::bump_instance`, from every state-changing function.

**Persistent** — everything else. Each write extends the entry to
`PERSISTENT_TTL_LEDGERS = 535_680` (~31 days at 5s) in the same call, via
`storage::bump_persistent`. A missing `extend_ttl` is the classic Soroban bug:
the entry expires and the accounting silently vanishes. No entries use
temporary storage.

`CreditedTotal` and `VestingReserved` are the liability ledger behind the sweep
cap: `execute_sweep` may move at most `balance(vault) - CreditedTotal -
VestingReserved`.

TTL additions:

- Counts are per-entry: `CycleData`, `CycleClaimed`, `Balance`, `Vesting` and
  `SweepRequest` each carry their own clock. `FixedShares` is bumped only when
  `configure_fixed_shares` runs.
- Only writes bump. A read of a 31-day-old `CycleData` does not refresh it, and
  a cycle whose claims have not started within that window can lapse.
- `SweepRequest` is persistent rather than temporary because temporary entries
  have a Mainnet maximum TTL of 17 280 ledgers (~1 day), shorter than the 72-hour
  sweep timelock. A request in temporary storage would expire before it could be
  executed.
