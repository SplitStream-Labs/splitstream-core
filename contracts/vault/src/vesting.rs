//! Linear vesting streams for maintainer retention rewards. Claims pay out
//! immediately from the contract's token balance (a separate accounting track
//! from the `Balance` pull-payment balances).

use soroban_sdk::{Address, Env, MuxedAddress, Symbol};

use crate::errors::SplitStreamError;
use crate::storage;
use crate::types::VestingData;

/// Create (or re-create) a linear vesting schedule for a contributor.
///
/// Re-creating a schedule preserves `claimed` from any existing schedule so a
/// re-created schedule can never let a contributor re-claim already-vested
/// funds; only the total, start and duration change.
pub(crate) fn create_vesting(
    env: &Env,
    contributor: &Address,
    total: i128,
    duration_ledgers: u32,
) -> Result<(), SplitStreamError> {
    let admin = storage::get_admin(env)?;
    admin.require_auth();
    storage::bump_instance(env);

    if total <= 0 || duration_ledgers == 0 {
        return Err(SplitStreamError::InvalidAmount);
    }

    let existing = storage::get_vesting(env, contributor);
    let existing_claimed = existing.as_ref().map(|v| v.claimed).unwrap_or(0);

    // A re-created schedule may never drop below what has already been paid
    // out; otherwise `total - claimed` would be negative and the next claim
    // would underflow the liability accounting.
    if total < existing_claimed {
        return Err(SplitStreamError::InvalidAmount);
    }

    let schedule = VestingData {
        total,
        claimed: existing_claimed,
        start_ledger: env.ledger().sequence(),
        duration_ledgers,
    };
    // Move the outstanding liability from the old remaining amount to the new
    // one in a single step, so a re-created schedule cannot double-count.
    let old_remaining = existing
        .map(|v| v.total.saturating_sub(v.claimed))
        .unwrap_or(0);
    let new_remaining = total - existing_claimed;
    storage::adjust_vesting_reserved(env, new_remaining - old_remaining)?;
    storage::set_vesting(env, contributor, &schedule);

    env.events().publish(
        (
            Symbol::new(env, "vesting_created"),
            contributor.clone(),
            total,
            duration_ledgers,
        ),
        (),
    );
    Ok(())
}

/// Claim the newly vested amount and transfer it directly to the contributor.
///
/// Returns the amount claimed; `0` is a legitimate "nothing new yet" state and
/// is not an error.
pub(crate) fn claim_vested(env: &Env, contributor: &Address) -> Result<i128, SplitStreamError> {
    contributor.require_auth();
    storage::bump_instance(env);

    let token = storage::get_token(env)?;
    let mut schedule =
        storage::get_vesting(env, contributor).ok_or(SplitStreamError::NoVestingSchedule)?;

    let elapsed = env
        .ledger()
        .sequence()
        .saturating_sub(schedule.start_ledger)
        .min(schedule.duration_ledgers);

    let vested = schedule
        .total
        .checked_mul(i128::from(elapsed))
        .and_then(|v| v.checked_div(i128::from(schedule.duration_ledgers)))
        .ok_or(SplitStreamError::InvalidAmount)?;

    // Saturating: `vested` can legitimately sit below `claimed` right after a
    // schedule is re-created (claimed is preserved, the new schedule starts at
    // zero elapsed). That is "nothing new to claim", not an underflow.
    let claimable = vested.saturating_sub(schedule.claimed);
    if claimable <= 0 {
        return Ok(0);
    }

    schedule.claimed = schedule
        .claimed
        .checked_add(claimable)
        .ok_or(SplitStreamError::InvalidAmount)?;
    storage::adjust_vesting_reserved(env, -claimable)?;
    storage::set_vesting(env, contributor, &schedule);

    let contract = env.current_contract_address();
    let to: MuxedAddress = contributor.into();
    soroban_sdk::token::TokenClient::new(env, &token).transfer(&contract, &to, &claimable);

    env.events().publish(
        (
            Symbol::new(env, "vested_claimed"),
            contributor.clone(),
            claimable,
        ),
        (),
    );
    Ok(claimable)
}
