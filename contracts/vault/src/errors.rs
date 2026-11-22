//! Contract errors for `SplitStreamVault`.

use soroban_sdk::contracterror;

/// Errors returned by the vault. The numeric discriminants are part of the
/// contract's on-chain ABI and must not be renumbered.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum SplitStreamError {
    /// The contract has not been initialized.
    NotInitialized = 1,
    /// The contract has already been initialized.
    AlreadyInitialized = 2,
    /// The caller is not authorized for this action.
    Unauthorized = 3,
    /// A root has already been posted for this cycle.
    CycleAlreadyPosted = 4,
    /// No cycle exists for this id.
    CycleNotFound = 5,
    /// The contributor has already claimed for this cycle.
    AlreadyClaimed = 6,
    /// The submitted Merkle proof does not match the stored root.
    InvalidProof = 7,
    /// No claimable balance to withdraw.
    InsufficientBalance = 8,
    /// The fixed-share basis points do not sum to 10_000.
    InvalidShareTotal = 9,
    /// The sweep timelock has not elapsed yet.
    SweepNotReady = 10,
    /// There is no pending sweep request.
    NoSweepPending = 11,
    /// The amount is not valid for this operation.
    InvalidAmount = 12,
    /// The challenge window has not elapsed; claims are not yet open.
    ClaimsNotYetOpen = 13,
    /// The challenge window has closed; the root can no longer be replaced.
    ChallengeWindowClosed = 14,
    /// The root for this cycle has already been replaced once.
    RootAlreadyReplaced = 15,
    /// Claims have already started against this cycle's root.
    ClaimsAlreadyStarted = 16,
    /// The contributor has no vesting schedule to claim against.
    NoVestingSchedule = 17,
    /// The sweep would take funds reserved for credited balances or vesting.
    InsufficientFreeFunds = 18,
    /// Claims for this cycle would exceed the posted `total_amount`.
    CycleTotalExceeded = 19,
}
