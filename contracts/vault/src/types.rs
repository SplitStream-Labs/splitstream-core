//! Data types shared by the vault's storage and logic modules.

use soroban_sdk::{contracttype, Address, BytesN};

/// Challenge-window state for one payout cycle.
///
/// A posted root is not immediately claimable: maintainers have a fixed
/// [`CHALLENGE_WINDOW_SECS`] window to catch a bad points calculation and
/// correct it on-chain (once) before any contributor can extract funds against
/// it.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CycleInfo {
    /// Merkle root of the payout manifest for this cycle.
    pub root: BytesN<32>,
    /// Total amount the manifest allocates for this cycle.
    pub total_amount: i128,
    /// Ledger timestamp (seconds) the root was posted; the challenge window is
    /// measured from this instant and does not restart on replacement.
    pub posted_at: u64,
    /// True after the first successful `credit_claim` — locks out replacement.
    pub claims_started: bool,
    /// True once the root has been challenged and replaced (one replacement max).
    pub replaced: bool,
}

/// Dispute window before claims open, in seconds (24 hours).
pub const CHALLENGE_WINDOW_SECS: u64 = 86_400;

/// Linear vesting schedule for a maintainer retention reward.
///
/// `claimed` is preserved across schedule re-creation so a replaced schedule
/// can never let a contributor re-claim already-vested funds.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VestingData {
    /// Total amount vested over the duration.
    pub total: i128,
    /// Amount already claimed.
    pub claimed: i128,
    /// Ledger sequence at which vesting started.
    pub start_ledger: u32,
    /// Vesting duration in ledgers.
    pub duration_ledgers: u32,
}

/// A pending sweep request subject to the sweep timelock.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SweepRequestData {
    /// Destination of the sweep.
    pub to: Address,
    /// Amount to sweep.
    pub amount: i128,
    /// Ledger timestamp (seconds) the request was made; the timelock is
    /// measured from this instant.
    pub requested_at: u64,
}
