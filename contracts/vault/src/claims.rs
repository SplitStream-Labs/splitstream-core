//! Pull-payment claim path: `credit_claim` credits a claimable balance after
//! Merkle-proof and challenge-window checks; `withdraw` pays it out.

use soroban_sdk::{Address, BytesN, Env, MuxedAddress, Symbol, Vec};

use crate::errors::SplitStreamError;
use crate::merkle;
use crate::storage;
use crate::types::CHALLENGE_WINDOW_SECS;

/// Credit a contributor's claimable balance for a cycle.
///
/// Enforcement order: cycle exists → challenge window elapsed → not already
/// claimed → proof verifies against the currently stored root (post-replacement
/// if one occurred). The first successful claim locks the root in place.
pub(crate) fn credit_claim(
    env: &Env,
    contributor: &Address,
    cycle_id: u64,
    amount: i128,
    proof: &Vec<BytesN<32>>,
) -> Result<(), SplitStreamError> {
    contributor.require_auth();
    // Hot claim paths must also refresh the instance TTL: `Admin`, `Oracle`
    // and `Token` live there, and a lapsed instance bricks every entry point.
    storage::bump_instance(env);

    let mut cycle =
        storage::get_cycle_info(env, cycle_id).ok_or(SplitStreamError::CycleNotFound)?;

    if amount <= 0 {
        return Err(SplitStreamError::InvalidAmount);
    }

    // Enforcement point for the dispute window — a posted root is not
    // claimable until the full challenge window has elapsed.
    let elapsed = env.ledger().timestamp().saturating_sub(cycle.posted_at);
    if elapsed < CHALLENGE_WINDOW_SECS {
        return Err(SplitStreamError::ClaimsNotYetOpen);
    }

    if storage::get_cycle_claimed(env, cycle_id, contributor) {
        return Err(SplitStreamError::AlreadyClaimed);
    }

    let leaf = merkle::leaf_hash(env, contributor, amount);
    if !merkle::verify_proof(env, &leaf, proof, &cycle.root) {
        return Err(SplitStreamError::InvalidProof);
    }

    // Defense in depth: a manifest can never pay out more than the total the
    // oracle committed to for the cycle, even if the root's leaves sum higher.
    let claimed_so_far = storage::get_cycle_claimed_total(env, cycle_id);
    let claimed_total = claimed_so_far
        .checked_add(amount)
        .ok_or(SplitStreamError::InvalidAmount)?;
    if claimed_total > cycle.total_amount {
        return Err(SplitStreamError::CycleTotalExceeded);
    }

    // Success: lock the root, mark claimed, credit the balance.
    cycle.claims_started = true;
    storage::set_cycle_info(env, cycle_id, &cycle);
    storage::set_cycle_claimed(env, cycle_id, contributor);
    storage::set_cycle_claimed_total(env, cycle_id, claimed_total);
    storage::add_balance(env, contributor, amount)?;

    env.events().publish(
        (
            Symbol::new(env, "claim_credited"),
            contributor.clone(),
            cycle_id,
            amount,
        ),
        (),
    );
    Ok(())
}

/// Withdraw the contributor's full claimable balance.
///
/// Checks-effects-interactions: the stored balance is zeroed before the token
/// transfer so a re-entrant call cannot double-withdraw.
pub(crate) fn withdraw(env: &Env, contributor: &Address) -> Result<(), SplitStreamError> {
    contributor.require_auth();

    let token = storage::get_token(env)?;
    storage::bump_instance(env);

    // Checks-effects: zero the balance and drop the liability before the
    // token transfer, so a re-entrant call sees neither.
    let balance = storage::take_balance(env, contributor)?;

    let contract = env.current_contract_address();
    let to: MuxedAddress = contributor.into();
    soroban_sdk::token::TokenClient::new(env, &token).transfer(&contract, &to, &balance);

    env.events().publish(
        (Symbol::new(env, "withdrawn"), contributor.clone(), balance),
        (),
    );
    Ok(())
}
