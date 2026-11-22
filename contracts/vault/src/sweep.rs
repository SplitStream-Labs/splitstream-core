//! Admin-initiated sweep of pooled funds to a destination, protected by a
//! 72-hour timelock measured in ledger timestamp seconds (not sequence
//! numbers) so a compromised admin key cannot instantly drain the pool.

use soroban_sdk::{Address, Env, MuxedAddress, Symbol};

use crate::errors::SplitStreamError;
use crate::storage;
use crate::types::SweepRequestData;

/// Timelock before a sweep can be executed, in seconds (72 hours).
pub const SWEEP_TIMELOCK_SECS: u64 = 259_200;

/// Register (or overwrite) a pending sweep request. Overwriting is allowed —
/// admin-controlled — but the timelock restarts from the new request time.
pub(crate) fn request_sweep(env: &Env, to: &Address, amount: i128) -> Result<(), SplitStreamError> {
    let admin = storage::get_admin(env)?;
    admin.require_auth();
    storage::bump_instance(env);

    if amount <= 0 {
        return Err(SplitStreamError::InvalidAmount);
    }

    let request = SweepRequestData {
        to: to.clone(),
        amount,
        requested_at: env.ledger().timestamp(),
    };
    storage::set_sweep_request(env, &request);

    env.events().publish(
        (Symbol::new(env, "sweep_requested"), to.clone(), amount),
        (),
    );
    Ok(())
}

/// Execute the pending sweep once the timelock has elapsed, then clear it.
pub(crate) fn execute_sweep(env: &Env) -> Result<(), SplitStreamError> {
    let admin = storage::get_admin(env)?;
    admin.require_auth();
    storage::bump_instance(env);

    let request = storage::get_sweep_request(env)?;

    let elapsed = env
        .ledger()
        .timestamp()
        .saturating_sub(request.requested_at);
    if elapsed < SWEEP_TIMELOCK_SECS {
        return Err(SplitStreamError::SweepNotReady);
    }

    let token = storage::get_token(env)?;
    let contract = env.current_contract_address();
    let token_client = soroban_sdk::token::TokenClient::new(env, &token);

    // The sweep may only move free funds: everything above the balances credited
    // to contributors and the outstanding vesting liability. Without this, an
    // admin sweep could strand claims that the vault already owes.
    let on_hand = token_client.balance(&contract);
    let available = on_hand.saturating_sub(storage::get_reserved_total(env)?);
    if request.amount > available {
        return Err(SplitStreamError::InsufficientFreeFunds);
    }

    let to: MuxedAddress = (&request.to).into();
    token_client.transfer(&contract, &to, &request.amount);
    storage::clear_sweep_request(env);

    env.events().publish(
        (
            Symbol::new(env, "sweep_executed"),
            request.to.clone(),
            request.amount,
        ),
        (),
    );
    Ok(())
}

/// Cancel any pending sweep request. Idempotent — no error when none is
/// pending, and no event is emitted because no state changes.
pub(crate) fn cancel_sweep(env: &Env) -> Result<(), SplitStreamError> {
    let admin = storage::get_admin(env)?;
    admin.require_auth();
    storage::bump_instance(env);

    if storage::clear_sweep_request(env) {
        env.events()
            .publish((Symbol::new(env, "sweep_cancelled"),), ());
    }
    Ok(())
}
