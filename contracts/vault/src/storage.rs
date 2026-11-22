//! Typed storage access for `SplitStreamVault`.
//!
//! Storage layout:
//! - `Admin`, `Oracle`, `Token` live in **instance** storage and are small and
//!   always-needed; their TTL is bumped on every admin-gated call.
//! - `CycleData`, `CycleClaimed`, `Balance`, `FixedShares`, `Vesting` and
//!   `SweepRequest` live in **persistent** storage and have their TTL extended
//!   on every write — a missing `extend_ttl` here is a classic Soroban bug
//!   (the entry expires and the accounting silently vanishes).
//!
//! `SweepRequest` is persistent rather than temporary (the storage-design
//! sketch put it in temporary storage) because temporary entries have a
//! Mainnet maximum TTL of 17,280 ledgers (~1 day), which is shorter than the
//! 72-hour sweep timelock (`SWEEP_TIMELOCK_SECS`). A request stored in
//! temporary storage would expire before it could be executed.

use soroban_sdk::{contracttype, Address, Env, Vec};

use crate::errors::SplitStreamError;
use crate::types::{CycleInfo, SweepRequestData, VestingData};

/// Ledger keys used by the vault.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// Admin address — instance storage.
    Admin,
    /// Oracle (CI relay) address — instance storage.
    Oracle,
    /// SEP-41 token contract address — instance storage.
    Token,
    /// Per-cycle challenge state — persistent storage.
    CycleData(u64),
    /// Per-cycle, per-contributor double-claim guard — persistent storage.
    CycleClaimed(u64, Address),
    /// Pull-payment claimable balance — persistent storage.
    Balance(Address),
    /// Sum of all outstanding pull-payment balances — instance storage.
    CreditedTotal,
    /// Outstanding (unvested-but-owned) vesting liability — instance storage.
    VestingReserved,
    /// Cumulative amount claimed against a cycle — persistent storage.
    CycleClaimedTotal(u64),
    /// Fixed basis-point split list (sums to 10_000) — persistent storage.
    FixedShares,
    /// Linear vesting schedule — persistent storage.
    Vesting(Address),
    /// Pending sweep request — persistent storage (see module docs).
    SweepRequest,
}

/// TTL target for the contract instance and code, in ledgers (~1 year at 5s).
pub const INSTANCE_TTL_LEDGERS: u32 = 6_311_520;
/// TTL target for persistent entries, in ledgers (~31 days at 5s).
pub const PERSISTENT_TTL_LEDGERS: u32 = 535_680;

/// Bump the contract instance/code TTL. Called on every admin-gated function.
pub fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_LEDGERS, INSTANCE_TTL_LEDGERS);
}

/// Extend the TTL of a persistent entry to `PERSISTENT_TTL_LEDGERS` if it is
/// currently below that threshold. Must be called in the same function that
/// performs the write.
pub fn bump_persistent(env: &Env, key: &DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, PERSISTENT_TTL_LEDGERS, PERSISTENT_TTL_LEDGERS);
}

/// True once `initialize` has stored the admin.
pub fn is_initialized(env: &Env) -> bool {
    env.storage().instance().has(&DataKey::Admin)
}

pub fn get_admin(env: &Env) -> Result<Address, SplitStreamError> {
    env.storage()
        .instance()
        .get::<DataKey, Address>(&DataKey::Admin)
        .ok_or(SplitStreamError::NotInitialized)
}

pub fn set_admin(env: &Env, admin: &Address) {
    env.storage().instance().set(&DataKey::Admin, admin);
}

pub fn get_oracle(env: &Env) -> Result<Address, SplitStreamError> {
    env.storage()
        .instance()
        .get::<DataKey, Address>(&DataKey::Oracle)
        .ok_or(SplitStreamError::NotInitialized)
}

pub fn set_oracle(env: &Env, oracle: &Address) {
    env.storage().instance().set(&DataKey::Oracle, oracle);
}

pub fn get_token(env: &Env) -> Result<Address, SplitStreamError> {
    env.storage()
        .instance()
        .get::<DataKey, Address>(&DataKey::Token)
        .ok_or(SplitStreamError::NotInitialized)
}

pub fn set_token(env: &Env, token: &Address) {
    env.storage().instance().set(&DataKey::Token, token);
}

/// Total of every contributor's outstanding pull-payment balance.
pub fn get_credited_total(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get::<DataKey, i128>(&DataKey::CreditedTotal)
        .unwrap_or(0)
}

pub fn set_credited_total(env: &Env, amount: i128) {
    env.storage()
        .instance()
        .set(&DataKey::CreditedTotal, &amount);
}

/// Amount still owed to contributors under vesting schedules
/// (`sum(total - claimed)`). Read on every sweep so the admin can never
/// sweep funds that a contributor is entitled to claim.
pub fn get_vesting_reserved(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get::<DataKey, i128>(&DataKey::VestingReserved)
        .unwrap_or(0)
}

pub fn set_vesting_reserved(env: &Env, amount: i128) {
    env.storage()
        .instance()
        .set(&DataKey::VestingReserved, &amount);
}

/// Every unit of the token the vault owes to someone: credited balances plus
/// outstanding vesting liability. `execute_sweep` may only move funds above
/// this line.
pub fn get_reserved_total(env: &Env) -> Result<i128, SplitStreamError> {
    get_credited_total(env)
        .checked_add(get_vesting_reserved(env))
        .ok_or(SplitStreamError::InvalidAmount)
}

/// Adjust the outstanding vesting liability by `delta` (positive when a
/// schedule is created or enlarged, negative when a contribution vests).
pub fn adjust_vesting_reserved(env: &Env, delta: i128) -> Result<(), SplitStreamError> {
    let updated = get_vesting_reserved(env)
        .checked_add(delta)
        .ok_or(SplitStreamError::InvalidAmount)?;
    if updated < 0 {
        return Err(SplitStreamError::InvalidAmount);
    }
    set_vesting_reserved(env, updated);
    Ok(())
}

pub fn get_cycle_info(env: &Env, cycle_id: u64) -> Option<CycleInfo> {
    env.storage()
        .persistent()
        .get::<DataKey, CycleInfo>(&DataKey::CycleData(cycle_id))
}

/// Write a cycle and extend its TTL in the same call.
pub fn set_cycle_info(env: &Env, cycle_id: u64, info: &CycleInfo) {
    let key = DataKey::CycleData(cycle_id);
    env.storage().persistent().set(&key, info);
    bump_persistent(env, &key);
}

/// Returns true if the contributor already claimed for this cycle.
pub fn get_cycle_claimed(env: &Env, cycle_id: u64, contributor: &Address) -> bool {
    env.storage()
        .persistent()
        .get::<DataKey, bool>(&DataKey::CycleClaimed(cycle_id, contributor.clone()))
        .unwrap_or(false)
}

/// Mark a contributor as claimed for a cycle and extend the TTL.
pub fn set_cycle_claimed(env: &Env, cycle_id: u64, contributor: &Address) {
    let key = DataKey::CycleClaimed(cycle_id, contributor.clone());
    env.storage().persistent().set(&key, &true);
    bump_persistent(env, &key);
}

/// Claimable pull-payment balance; 0 when no balance has ever been credited.
pub fn get_balance(env: &Env, contributor: &Address) -> i128 {
    env.storage()
        .persistent()
        .get::<DataKey, i128>(&DataKey::Balance(contributor.clone()))
        .unwrap_or(0)
}

/// Overwrite a balance and extend the TTL.
pub fn set_balance(env: &Env, contributor: &Address, amount: i128) {
    let key = DataKey::Balance(contributor.clone());
    env.storage().persistent().set(&key, &amount);
    bump_persistent(env, &key);
}

/// Credit a balance by `delta` (which may be negative when a distribution is
/// corrected), returning the new balance. Extends the TTL on write.
pub fn add_balance(
    env: &Env,
    contributor: &Address,
    delta: i128,
) -> Result<i128, SplitStreamError> {
    let current = get_balance(env, contributor);
    let new_balance = current
        .checked_add(delta)
        .ok_or(SplitStreamError::InvalidAmount)?;
    let new_total = get_credited_total(env)
        .checked_add(delta)
        .ok_or(SplitStreamError::InvalidAmount)?;
    if new_balance < 0 || new_total < 0 {
        return Err(SplitStreamError::InvalidAmount);
    }
    set_balance(env, contributor, new_balance);
    set_credited_total(env, new_total);
    Ok(new_balance)
}

/// Zero a contributor's balance and return the amount removed, decreasing the
/// outstanding credited total by the same amount. Used by `withdraw` so the
/// liability and the balance can never drift apart.
pub fn take_balance(env: &Env, contributor: &Address) -> Result<i128, SplitStreamError> {
    let balance = get_balance(env, contributor);
    if balance <= 0 {
        return Err(SplitStreamError::InsufficientBalance);
    }
    let new_total = get_credited_total(env)
        .checked_sub(balance)
        .ok_or(SplitStreamError::InvalidAmount)?;
    if new_total < 0 {
        return Err(SplitStreamError::InvalidAmount);
    }
    set_balance(env, contributor, 0);
    set_credited_total(env, new_total);
    Ok(balance)
}

/// Cumulative amount credited against a cycle's manifest.
pub fn get_cycle_claimed_total(env: &Env, cycle_id: u64) -> i128 {
    env.storage()
        .persistent()
        .get::<DataKey, i128>(&DataKey::CycleClaimedTotal(cycle_id))
        .unwrap_or(0)
}

/// Set the cumulative claimed amount for a cycle and extend its TTL.
pub fn set_cycle_claimed_total(env: &Env, cycle_id: u64, amount: i128) {
    let key = DataKey::CycleClaimedTotal(cycle_id);
    env.storage().persistent().set(&key, &amount);
    bump_persistent(env, &key);
}

/// The configured fixed split list; empty when never configured.
pub fn get_fixed_shares(env: &Env) -> Vec<(Address, u32)> {
    env.storage()
        .persistent()
        .get::<DataKey, Vec<(Address, u32)>>(&DataKey::FixedShares)
        .unwrap_or_else(|| Vec::new(env))
}

/// Overwrite the fixed split list and extend the TTL.
pub fn set_fixed_shares(env: &Env, shares: &Vec<(Address, u32)>) {
    env.storage()
        .persistent()
        .set(&DataKey::FixedShares, shares);
    bump_persistent(env, &DataKey::FixedShares);
}

pub fn get_vesting(env: &Env, contributor: &Address) -> Option<VestingData> {
    env.storage()
        .persistent()
        .get::<DataKey, VestingData>(&DataKey::Vesting(contributor.clone()))
}

/// Write a vesting schedule and extend the TTL.
pub fn set_vesting(env: &Env, contributor: &Address, data: &VestingData) {
    let key = DataKey::Vesting(contributor.clone());
    env.storage().persistent().set(&key, data);
    bump_persistent(env, &key);
}

/// Load the pending sweep request, or `NoSweepPending` when none exists.
pub fn get_sweep_request(env: &Env) -> Result<SweepRequestData, SplitStreamError> {
    env.storage()
        .persistent()
        .get::<DataKey, SweepRequestData>(&DataKey::SweepRequest)
        .ok_or(SplitStreamError::NoSweepPending)
}

/// Write a sweep request and extend the TTL.
pub fn set_sweep_request(env: &Env, request: &SweepRequestData) {
    env.storage()
        .persistent()
        .set(&DataKey::SweepRequest, request);
    bump_persistent(env, &DataKey::SweepRequest);
}

/// Clear any pending sweep request; returns true if one existed.
pub fn clear_sweep_request(env: &Env) -> bool {
    let key = DataKey::SweepRequest;
    if env.storage().persistent().has(&key) {
        env.storage().persistent().remove(&key);
        true
    } else {
        false
    }
}
