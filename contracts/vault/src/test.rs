//! Test suites for SplitStreamVault.
//!
//! One module per feature: deposit/pull-payment claims, Merkle verification,
//! challenge-window semantics, fixed-split/vesting/sweep timelocks, and
//! per-address authorization.

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger as _, MockAuth, MockAuthInvoke},
    token, vec, Address, BytesN, Env, IntoVal, Symbol, Val, Vec,
};

use crate::errors::SplitStreamError;
use crate::merkle::{hash_pair, leaf_hash, verify_proof};
use crate::sweep::SWEEP_TIMELOCK_SECS;
use crate::types::CHALLENGE_WINDOW_SECS;
use crate::{SplitStreamVault, SplitStreamVaultClient};

/// Test fixture: an initialized vault backed by a mintable SAC.
///
/// Clients are constructed on demand (they borrow the `Env`), so the fixture
/// only holds addresses.
struct TestEnv {
    env: Env,
    admin: Address,
    oracle: Address,
    token: Address,
    contract_id: Address,
}

fn setup() -> TestEnv {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let oracle = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(admin.clone());
    let token = token_contract.address();
    let contract_id = env.register(SplitStreamVault, ());
    SplitStreamVaultClient::new(&env, &contract_id).initialize(&admin, &oracle, &token);
    TestEnv {
        env,
        admin,
        oracle,
        token,
        contract_id,
    }
}

impl TestEnv {
    fn client(&self) -> SplitStreamVaultClient<'_> {
        SplitStreamVaultClient::new(&self.env, &self.contract_id)
    }

    fn sac(&self) -> token::StellarAssetClient<'_> {
        token::StellarAssetClient::new(&self.env, &self.token)
    }

    fn token_balance(&self, addr: &Address) -> i128 {
        token::TokenClient::new(&self.env, &self.token).balance(addr)
    }
}

/// Build a balanced Merkle tree over manifest entries using the same
/// sorted-pair convention as the verifier, returning the root and per-entry
/// bottom-up proofs. `entries` must be a power-of-two count.
fn build_manifest(
    env: &Env,
    entries: &[(Address, i128)],
) -> (BytesN<32>, std::vec::Vec<(Address, i128, Vec<BytesN<32>>)>) {
    let mut leaves: std::vec::Vec<(Address, i128, BytesN<32>)> = entries
        .iter()
        .map(|(addr, amt)| (addr.clone(), *amt, leaf_hash(env, addr, *amt)))
        .collect();
    leaves.sort_by(|a, b| a.2.cmp(&b.2));
    let n = leaves.len();
    assert!(n.is_power_of_two(), "test manifests use power-of-two leaves");

    // Heap-style tree array: unused index 0, internal nodes at [1, n),
    // leaves at [n, 2n).
    let mut tree: std::vec::Vec<BytesN<32>> = std::vec::Vec::with_capacity(2 * n);
    tree.push(leaf_hash(env, &Address::generate(env), 0)); // unused index 0
    for _ in 1..n {
        tree.push(leaf_hash(env, &Address::generate(env), 0)); // internal [1, n)
    }
    for leaf in &leaves {
        tree.push(leaf.2.clone());
    }
    for i in (1..n).rev() {
        let (a, b) = (tree[2 * i].clone(), tree[2 * i + 1].clone());
        tree[i] = hash_pair(env, &a, &b);
    }
    let root = tree[1].clone();

    let mut result = std::vec::Vec::new();
    for (idx, (addr, amt, _)) in leaves.iter().enumerate() {
        let mut proof = std::vec::Vec::new();
        let mut node = n + idx;
        while node > 1 {
            let sibling = if node.is_multiple_of(2) {
                tree[node + 1].clone()
            } else {
                tree[node - 1].clone()
            };
            proof.push(sibling);
            node /= 2;
        }
        let mut proof_vec = Vec::new(env);
        for p in &proof {
            proof_vec.push_back(p.clone());
        }
        result.push((addr.clone(), *amt, proof_vec));
    }
    (root, result)
}

/// Look up the proof for a specific contributor.
fn proof_for<'a>(
    claims: &'a [(Address, i128, Vec<BytesN<32>>)],
    addr: &Address,
) -> &'a Vec<BytesN<32>> {
    claims
        .iter()
        .find(|(a, _, _)| a == addr)
        .map(|(_, _, p)| p)
        .expect("contributor present in manifest")
}

/// Mint tokens and deposit them into the vault.
fn fund_vault(t: &TestEnv, amount: i128) {
    t.sac().mint(&t.admin, &amount);
    t.client().deposit(&t.admin, &amount);
}
mod deposit_claims {
    use super::*;

    #[test]
    fn deposit_transfers_tokens_and_emits_event() {
        let t = setup();
        t.sac().mint(&t.admin, &10_000);
        t.client().deposit(&t.admin, &1_000);

        // Read events before any further invocation, since `events().all()`
        // only reports the last contract call, and filter to this contract
        // (the token's inner `transfer` event is also emitted).
        assert_eq!(
            t.env.events().all().filter_by_contract(&t.contract_id),
            vec![
                &t.env,
                (
                    t.contract_id.clone(),
                    (Symbol::new(&t.env, "deposit"), t.admin.clone(), 1_000_i128)
                        .into_val(&t.env),
                    ().into_val(&t.env),
                )
            ]
        );

        assert_eq!(t.token_balance(&t.contract_id), 1_000);
    }

    #[test]
    fn deposit_rejects_zero_or_negative_amounts() {
        let t = setup();
        assert_eq!(
            t.client().try_deposit(&t.admin, &0),
            Err(Ok(SplitStreamError::InvalidAmount))
        );
        assert_eq!(
            t.client().try_deposit(&t.admin, &-5),
            Err(Ok(SplitStreamError::InvalidAmount))
        );
    }

    #[test]
    fn initialize_rejects_second_call() {
        let t = setup();
        assert_eq!(
            t.client().try_initialize(&t.admin, &t.oracle, &t.token),
            Err(Ok(SplitStreamError::AlreadyInitialized))
        );
    }

    #[test]
    fn claim_credits_balance_and_withdraw_pays_out() {
        let t = setup();
        fund_vault(&t, 10_000);

        let (root, claims) =
            build_manifest(&t.env, &[(t.admin.clone(), 400), (t.oracle.clone(), 600)]);
        t.client().post_cycle_root(&1, &root, &1_000);
        t.env.ledger().set_timestamp(CHALLENGE_WINDOW_SECS);

        t.client()
            .credit_claim(&t.admin, &1, &400, proof_for(&claims, &t.admin));
        assert_eq!(t.client().get_balance(&t.admin), 400);

        t.client().withdraw(&t.admin);
        assert_eq!(t.client().get_balance(&t.admin), 0);
        assert_eq!(t.token_balance(&t.admin), 400);
    }

    #[test]
    fn withdraw_rejects_empty_balance() {
        let t = setup();
        assert_eq!(
            t.client().try_withdraw(&t.admin),
            Err(Ok(SplitStreamError::InsufficientBalance))
        );
    }

    #[test]
    fn claim_rejects_double_claim() {
        let t = setup();
        let (root, claims) = build_manifest(&t.env, &[(t.admin.clone(), 100)]);
        t.client().post_cycle_root(&1, &root, &100);
        t.env.ledger().set_timestamp(CHALLENGE_WINDOW_SECS);
        t.client()
            .credit_claim(&t.admin, &1, &100, proof_for(&claims, &t.admin));
        assert_eq!(
            t.client()
                .try_credit_claim(&t.admin, &1, &100, proof_for(&claims, &t.admin)),
            Err(Ok(SplitStreamError::AlreadyClaimed))
        );
        assert!(t.client().has_claimed(&1, &t.admin));
    }

    #[test]
    fn claim_rejects_unknown_cycle_and_bad_proof() {
        let t = setup();
        let (root, claims) = build_manifest(&t.env, &[(t.admin.clone(), 100)]);
        t.client().post_cycle_root(&1, &root, &100);
        t.env.ledger().set_timestamp(CHALLENGE_WINDOW_SECS);

        assert_eq!(
            t.client()
                .try_credit_claim(&t.admin, &99, &100, proof_for(&claims, &t.admin)),
            Err(Ok(SplitStreamError::CycleNotFound))
        );
        let wrong = vec![&t.env, BytesN::from_array(&t.env, &[0xAB; 32])];
        assert_eq!(
            t.client().try_credit_claim(&t.admin, &1, &100, &wrong),
            Err(Ok(SplitStreamError::InvalidProof))
        );
        assert_eq!(
            t.client()
                .try_credit_claim(&t.admin, &1, &101, proof_for(&claims, &t.admin)),
            Err(Ok(SplitStreamError::InvalidProof))
        );
    }
}
mod merkle_tests {
    use super::*;

    #[test]
    fn leaf_hash_is_deterministic_and_distinct() {
        let env = Env::default();
        let a = Address::generate(&env);
        let b = Address::generate(&env);
        let h1 = leaf_hash(&env, &a, 100);
        assert_eq!(h1, leaf_hash(&env, &a, 100));
        assert_ne!(h1, leaf_hash(&env, &a, 101));
        assert_ne!(h1, leaf_hash(&env, &b, 100));
    }

    #[test]
    fn proof_verifies_for_every_leaf_including_right_sided() {
        let env = Env::default();
        let a = Address::generate(&env);
        let b = Address::generate(&env);
        let c = Address::generate(&env);
        let d = Address::generate(&env);
        let (root, claims) = build_manifest(&env, &[(a, 100), (b, 200), (c, 300), (d, 400)]);
        for (addr, amt, proof) in &claims {
            let leaf = leaf_hash(&env, addr, *amt);
            assert!(verify_proof(&env, &leaf, proof, &root), "leaf {:?}", addr);
        }
    }

    #[test]
    fn tampered_proofs_are_rejected() {
        let env = Env::default();
        let a = Address::generate(&env);
        let b = Address::generate(&env);
        let (root, claims) = build_manifest(&env, &[(a.clone(), 100), (b.clone(), 200)]);
        let proof_a = proof_for(&claims, &a);

        // Wrong amount produces a leaf that does not verify.
        let wrong_leaf = leaf_hash(&env, &a, 101);
        assert!(!verify_proof(&env, &wrong_leaf, proof_a, &root));

        // Swapping in the sibling of another leaf fails.
        let proof_b = proof_for(&claims, &b);
        let leaf_a = leaf_hash(&env, &a, 100);
        assert!(!verify_proof(&env, &leaf_a, proof_b, &root));

        // An arbitrary root rejects every proof.
        let fake_root = BytesN::from_array(&env, &[0x42; 32]);
        assert!(!verify_proof(&env, &leaf_a, proof_a, &fake_root));
    }

    #[test]
    fn single_leaf_manifest_root_is_the_leaf() {
        let env = Env::default();
        let a = Address::generate(&env);
        let (root, claims) = build_manifest(&env, &[(a.clone(), 100)]);
        assert_eq!(root, leaf_hash(&env, &a, 100));
        assert!(claims[0].2.is_empty());
    }
}
mod challenge_window {
    use super::*;

    #[test]
    fn claim_rejected_until_window_elapses() {
        let t = setup();
        let (root, claims) = build_manifest(&t.env, &[(t.admin.clone(), 100)]);
        t.env.ledger().set_timestamp(1_000_000);
        t.client().post_cycle_root(&1, &root, &100);

        // Window boundary is inclusive: 86_399s after posting is still closed.
        t.env
            .ledger()
            .set_timestamp(1_000_000 + CHALLENGE_WINDOW_SECS - 1);
        assert_eq!(
            t.client()
                .try_credit_claim(&t.admin, &1, &100, proof_for(&claims, &t.admin)),
            Err(Ok(SplitStreamError::ClaimsNotYetOpen))
        );

        // Exactly at 86_400s claims open.
        t.env
            .ledger()
            .set_timestamp(1_000_000 + CHALLENGE_WINDOW_SECS);
        t.client()
            .credit_claim(&t.admin, &1, &100, proof_for(&claims, &t.admin));
        assert_eq!(t.client().get_balance(&t.admin), 100);
    }

    #[test]
    fn replace_within_window_overrides_root_and_keeps_posted_at() {
        let t = setup();
        let (root1, claims1) = build_manifest(&t.env, &[(t.admin.clone(), 100)]);
        let (root2, claims2) = build_manifest(&t.env, &[(t.admin.clone(), 200)]);
        t.env.ledger().set_timestamp(1_000_000);
        t.client().post_cycle_root(&1, &root1, &100);

        t.client().challenge_and_replace_root(&1, &root2, &200);

        let info = t.client().get_cycle_info(&1).unwrap();
        assert_eq!(info.root, root2);
        assert_eq!(info.total_amount, 200);
        assert!(info.replaced);
        assert!(!info.claims_started);
        // The dispute window does not restart on replacement.
        assert_eq!(info.posted_at, 1_000_000);

        // Claims must now prove against the NEW root.
        t.env
            .ledger()
            .set_timestamp(1_000_000 + CHALLENGE_WINDOW_SECS);
        assert_eq!(
            t.client()
                .try_credit_claim(&t.admin, &1, &100, proof_for(&claims1, &t.admin)),
            Err(Ok(SplitStreamError::InvalidProof))
        );
        t.client()
            .credit_claim(&t.admin, &1, &200, proof_for(&claims2, &t.admin));
        assert_eq!(t.client().get_balance(&t.admin), 200);
    }

    #[test]
    fn replacement_rejected_after_claims_started() {
        let t = setup();
        let (root, claims) = build_manifest(&t.env, &[(t.admin.clone(), 100)]);
        t.env.ledger().set_timestamp(1_000_000);
        t.client().post_cycle_root(&1, &root, &100);
        t.env
            .ledger()
            .set_timestamp(1_000_000 + CHALLENGE_WINDOW_SECS);
        t.client()
            .credit_claim(&t.admin, &1, &100, proof_for(&claims, &t.admin));

        let (root2, _) = build_manifest(&t.env, &[(t.admin.clone(), 200)]);
        // claims_started takes precedence even though the window has also closed.
        assert_eq!(
            t.client().try_challenge_and_replace_root(&1, &root2, &200),
            Err(Ok(SplitStreamError::ClaimsAlreadyStarted))
        );
    }

    #[test]
    fn replacement_rejected_after_window_closes() {
        let t = setup();
        let (root, _) = build_manifest(&t.env, &[(t.admin.clone(), 100)]);
        t.client().post_cycle_root(&1, &root, &100);
        t.env.ledger().set_timestamp(CHALLENGE_WINDOW_SECS);
        let (root2, _) = build_manifest(&t.env, &[(t.admin.clone(), 200)]);
        assert_eq!(
            t.client().try_challenge_and_replace_root(&1, &root2, &200),
            Err(Ok(SplitStreamError::ChallengeWindowClosed))
        );
    }

    #[test]
    fn second_replacement_rejected() {
        let t = setup();
        let (root1, _) = build_manifest(&t.env, &[(t.admin.clone(), 100)]);
        let (root2, _) = build_manifest(&t.env, &[(t.admin.clone(), 200)]);
        let (root3, _) = build_manifest(&t.env, &[(t.admin.clone(), 300)]);
        t.client().post_cycle_root(&1, &root1, &100);
        t.client().challenge_and_replace_root(&1, &root2, &200);
        assert_eq!(
            t.client().try_challenge_and_replace_root(&1, &root3, &300),
            Err(Ok(SplitStreamError::RootAlreadyReplaced))
        );
    }

    #[test]
    fn replacement_rejected_for_unknown_cycle() {
        let t = setup();
        let (root, _) = build_manifest(&t.env, &[(t.admin.clone(), 100)]);
        assert_eq!(
            t.client().try_challenge_and_replace_root(&42, &root, &100),
            Err(Ok(SplitStreamError::CycleNotFound))
        );
    }
}
mod fixed_vesting_sweep {
    use super::*;

    #[test]
    fn fixed_shares_must_sum_to_10000() {
        let t = setup();
        let bad = vec![&t.env, (t.admin.clone(), 9_998_u32), (t.oracle.clone(), 1_u32)];
        assert_eq!(
            t.client().try_configure_fixed_shares(&bad),
            Err(Ok(SplitStreamError::InvalidShareTotal))
        );
        let bad = vec![&t.env, (t.admin.clone(), 5_000_u32), (t.oracle.clone(), 5_001_u32)];
        assert_eq!(
            t.client().try_configure_fixed_shares(&bad),
            Err(Ok(SplitStreamError::InvalidShareTotal))
        );

        let good = vec![&t.env, (t.admin.clone(), 5_000_u32), (t.oracle.clone(), 5_000_u32)];
        t.client().configure_fixed_shares(&good);
        assert_eq!(t.client().get_fixed_shares(), good);
    }

    #[test]
    fn distribute_fixed_credits_basis_point_shares() {
        let t = setup();
        let shares = vec![
            &t.env,
            (t.admin.clone(), 5_000_u32),
            (t.oracle.clone(), 3_000_u32),
            (t.contract_id.clone(), 2_000_u32),
        ];
        t.client().configure_fixed_shares(&shares);
        t.client().distribute_fixed(&1_000);
        assert_eq!(t.client().get_balance(&t.admin), 500);
        assert_eq!(t.client().get_balance(&t.oracle), 300);
        assert_eq!(t.client().get_balance(&t.contract_id), 200);

        // Rounding truncates toward zero per recipient (integer math only).
        t.client().distribute_fixed(&9);
        assert_eq!(t.client().get_balance(&t.admin), 504);
        assert_eq!(t.client().get_balance(&t.oracle), 302);
        assert_eq!(t.client().get_balance(&t.contract_id), 201);
    }

    #[test]
    fn distribute_fixed_rejects_nonpositive_amount() {
        let t = setup();
        assert_eq!(
            t.client().try_distribute_fixed(&0),
            Err(Ok(SplitStreamError::InvalidAmount))
        );
        // The admin twin shares the same validation (same internal body).
        assert_eq!(
            t.client().try_admin_distribute_fixed(&0),
            Err(Ok(SplitStreamError::InvalidAmount))
        );
    }

    #[test]
    fn admin_distribute_fixed_credits_basis_point_shares() {
        let t = setup();
        let shares = vec![
            &t.env,
            (t.admin.clone(), 5_000_u32),
            (t.oracle.clone(), 3_000_u32),
            (t.contract_id.clone(), 2_000_u32),
        ];
        t.client().configure_fixed_shares(&shares);
        t.client().admin_distribute_fixed(&1_000);
        assert_eq!(t.client().get_balance(&t.admin), 500);
        assert_eq!(t.client().get_balance(&t.oracle), 300);
        assert_eq!(t.client().get_balance(&t.contract_id), 200);

        // Rounding truncates toward zero per recipient (integer math only),
        // exactly as on the oracle path.
        t.client().admin_distribute_fixed(&9);
        assert_eq!(t.client().get_balance(&t.admin), 504);
        assert_eq!(t.client().get_balance(&t.oracle), 302);
        assert_eq!(t.client().get_balance(&t.contract_id), 201);
    }

    #[test]
    fn distributed_balances_withdraw_after_funding() {
        let t = setup();
        fund_vault(&t, 10_000); // admin deposits their entire minted balance
        let shares = vec![&t.env, (t.admin.clone(), 10_000_u32)];
        t.client().configure_fixed_shares(&shares);
        t.client().distribute_fixed(&2_000);
        t.client().withdraw(&t.admin);
        // The 10_000 deposit went to the vault; the 2_000 distribution is
        // what comes back out on withdrawal.
        assert_eq!(t.token_balance(&t.admin), 2_000);
        assert_eq!(t.token_balance(&t.contract_id), 8_000);
    }

    #[test]
    fn vesting_releases_linearly_and_transfers_directly() {
        let t = setup();
        fund_vault(&t, 10_000);
        t.env.ledger().set_sequence_number(100);
        t.client().create_vesting(&t.admin, &1_000, &100);

        // No elapsed time yet — legitimate no-op returning 0.
        assert_eq!(t.client().claim_vested(&t.admin), 0);

        t.env.ledger().set_sequence_number(150); // 50% elapsed
        assert_eq!(t.client().claim_vested(&t.admin), 500);
        assert_eq!(t.token_balance(&t.admin), 500);

        // Nothing new until more time passes.
        assert_eq!(t.client().claim_vested(&t.admin), 0);

        t.env.ledger().set_sequence_number(200); // 100% elapsed (capped)
        assert_eq!(t.client().claim_vested(&t.admin), 500);
        assert_eq!(t.client().claim_vested(&t.admin), 0);
        assert_eq!(t.token_balance(&t.admin), 1_000);
    }

    #[test]
    fn vesting_recreate_preserves_claimed() {
        let t = setup();
        fund_vault(&t, 10_000);
        t.env.ledger().set_sequence_number(100);
        t.client().create_vesting(&t.admin, &1_000, &100);
        t.env.ledger().set_sequence_number(150);
        assert_eq!(t.client().claim_vested(&t.admin), 500);

        // Admin re-creates the schedule with a larger total — claimed survives.
        t.env.ledger().set_sequence_number(200);
        t.client().create_vesting(&t.admin, &2_000, &100);
        // 2_000 * 0 / 100 = 0 vested so far in the new schedule; 0 < 500 claimed.
        assert_eq!(t.client().claim_vested(&t.admin), 0);
        t.env.ledger().set_sequence_number(225); // 25% of new schedule = 500
        assert_eq!(t.client().claim_vested(&t.admin), 0);
        t.env.ledger().set_sequence_number(300); // fully vested = 2_000
        assert_eq!(t.client().claim_vested(&t.admin), 1_500);
    }

    #[test]
    fn vesting_rejects_invalid_params_and_missing_schedule() {
        let t = setup();
        assert_eq!(
            t.client().try_create_vesting(&t.admin, &0, &100),
            Err(Ok(SplitStreamError::InvalidAmount))
        );
        assert_eq!(
            t.client().try_create_vesting(&t.admin, &100, &0),
            Err(Ok(SplitStreamError::InvalidAmount))
        );
        assert_eq!(
            t.client().try_claim_vested(&t.admin),
            Err(Ok(SplitStreamError::NoVestingSchedule))
        );
    }

    #[test]
    fn claim_vested_reports_no_vesting_schedule_for_unknown_contributor() {
        let t = setup();
        let stranger = Address::generate(&t.env);
        assert_eq!(
            t.client().try_claim_vested(&stranger),
            Err(Ok(SplitStreamError::NoVestingSchedule))
        );
    }

    #[test]
    fn sweep_timelock_enforced_in_seconds() {
        let t = setup();
        fund_vault(&t, 10_000);
        t.env.ledger().set_timestamp(1_000_000);
        t.client().request_sweep(&t.oracle, &2_500);

        // One second before the timelock elapses it is still locked.
        t.env
            .ledger()
            .set_timestamp(1_000_000 + SWEEP_TIMELOCK_SECS - 1);
        assert_eq!(
            t.client().try_execute_sweep(),
            Err(Ok(SplitStreamError::SweepNotReady))
        );

        // Exactly at 72h the sweep executes and clears.
        t.env
            .ledger()
            .set_timestamp(1_000_000 + SWEEP_TIMELOCK_SECS);
        t.client().execute_sweep();
        assert_eq!(t.token_balance(&t.oracle), 2_500);
        assert_eq!(
            t.client().try_execute_sweep(),
            Err(Ok(SplitStreamError::NoSweepPending))
        );
    }

    #[test]
    fn sweep_request_overwrite_restarts_timelock() {
        let t = setup();
        fund_vault(&t, 10_000);
        t.env.ledger().set_timestamp(1_000_000);
        t.client().request_sweep(&t.oracle, &1_000);
        // Overwrite with a different destination/amount.
        t.env.ledger().set_timestamp(1_000_000 + 10);
        t.client().request_sweep(&t.admin, &2_000);

        // The old request's timelock does not apply to the new one.
        t.env
            .ledger()
            .set_timestamp(1_000_000 + SWEEP_TIMELOCK_SECS);
        assert_eq!(
            t.client().try_execute_sweep(),
            Err(Ok(SplitStreamError::SweepNotReady))
        );
        t.env
            .ledger()
            .set_timestamp(1_000_000 + 10 + SWEEP_TIMELOCK_SECS);
        t.client().execute_sweep();
        assert_eq!(t.token_balance(&t.admin), 2_000);
    }

    #[test]
    fn sweep_cancel_is_idempotent_and_clears_request() {
        let t = setup();
        t.client().request_sweep(&t.oracle, &100);
        t.client().cancel_sweep();
        assert_eq!(
            t.client().try_execute_sweep(),
            Err(Ok(SplitStreamError::NoSweepPending))
        );
        // Cancelling again with nothing pending is a no-op, not an error.
        t.client().cancel_sweep();
    }

    #[test]
    fn sweep_rejects_nonpositive_amount() {
        let t = setup();
        assert_eq!(
            t.client().try_request_sweep(&t.oracle, &0),
            Err(Ok(SplitStreamError::InvalidAmount))
        );
    }
}
mod auth {
    use super::*;

    fn auth_env() -> (Env, SplitStreamVaultClient<'static>, Address, Address) {
        let env = Env::default();
        let admin = Address::generate(&env);
        let oracle = Address::generate(&env);
        let token = Address::generate(&env);
        env.mock_all_auths();
        let contract_id = env.register(SplitStreamVault, ());
        let client = SplitStreamVaultClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &token);
        (env, client, admin, oracle)
    }

    fn mock_one(
        env: &Env,
        contract_id: &Address,
        address: &Address,
        fn_name: &str,
        args: Vec<Val>,
    ) {
        env.mock_auths(&[MockAuth {
            address,
            invoke: &MockAuthInvoke {
                contract: contract_id,
                fn_name,
                args,
                sub_invokes: &[],
            },
        }]);
    }

    #[test]
    fn post_cycle_root_requires_oracle_not_admin() {
        let (env, client, admin, oracle) = auth_env();
        let root = BytesN::from_array(&env, &[0x11; 32]);
        let args: Vec<Val> = (1_u64, root.clone(), 1_000_i128).into_val(&env);
        let contract_id = client.address.clone();

        // Admin's auth must not authorize the oracle-gated function.
        mock_one(&env, &contract_id, &admin, "post_cycle_root", args.clone());
        assert!(client.try_post_cycle_root(&1, &root, &1_000).is_err());

        // Oracle's auth authorizes it.
        mock_one(&env, &contract_id, &oracle, "post_cycle_root", args);
        client.post_cycle_root(&1, &root, &1_000);
    }

    #[test]
    fn challenge_and_replace_requires_admin_not_oracle() {
        let (env, client, admin, oracle) = auth_env();
        let root = BytesN::from_array(&env, &[0x22; 32]);
        let contract_id = client.address.clone();
        mock_one(
            &env,
            &contract_id,
            &oracle,
            "post_cycle_root",
            (1_u64, root.clone(), 1_000_i128).into_val(&env),
        );
        client.post_cycle_root(&1, &root, &1_000);

        let new_root = BytesN::from_array(&env, &[0x33; 32]);
        let args: Vec<Val> = (1_u64, new_root.clone(), 2_000_i128).into_val(&env);
        // Oracle cannot replace its own root.
        mock_one(
            &env,
            &contract_id,
            &oracle,
            "challenge_and_replace_root",
            args.clone(),
        );
        assert!(client
            .try_challenge_and_replace_root(&1, &new_root, &2_000)
            .is_err());
        // Admin can.
        mock_one(
            &env,
            &contract_id,
            &admin,
            "challenge_and_replace_root",
            args,
        );
        client.challenge_and_replace_root(&1, &new_root, &2_000);
    }

    #[test]
    fn distribute_fixed_requires_oracle() {
        let (env, client, admin, oracle) = auth_env();
        let contract_id = client.address.clone();
        let shares = vec![&env, (admin.clone(), 10_000_u32)];
        mock_one(
            &env,
            &contract_id,
            &admin,
            "configure_fixed_shares",
            (shares.clone(),).into_val(&env),
        );
        client.configure_fixed_shares(&shares);

        let args: Vec<Val> = (1_000_i128,).into_val(&env);
        mock_one(&env, &contract_id, &admin, "distribute_fixed", args.clone());
        assert!(client.try_distribute_fixed(&1_000).is_err());
        mock_one(&env, &contract_id, &oracle, "distribute_fixed", args);
        client.distribute_fixed(&1_000);
    }

    #[test]
    fn admin_distribute_fixed_requires_admin_not_oracle() {
        let (env, client, admin, oracle) = auth_env();
        let contract_id = client.address.clone();
        let shares = vec![&env, (admin.clone(), 10_000_u32)];
        mock_one(
            &env,
            &contract_id,
            &admin,
            "configure_fixed_shares",
            (shares.clone(),).into_val(&env),
        );
        client.configure_fixed_shares(&shares);

        // Oracle's auth must not authorize the admin-gated function.
        let args: Vec<Val> = (1_000_i128,).into_val(&env);
        mock_one(
            &env,
            &contract_id,
            &oracle,
            "admin_distribute_fixed",
            args.clone(),
        );
        assert!(client.try_admin_distribute_fixed(&1_000).is_err());

        // Admin's auth authorizes it.
        mock_one(&env, &contract_id, &admin, "admin_distribute_fixed", args);
        client.admin_distribute_fixed(&1_000);
        assert_eq!(client.get_balance(&admin), 1_000);
    }

    #[test]
    fn admin_gated_functions_reject_other_callers() {
        let (env, client, admin, oracle) = auth_env();
        let contract_id = client.address.clone();

        // configure_fixed_shares with oracle auth fails.
        let shares = vec![&env, (admin.clone(), 10_000_u32)];
        mock_one(
            &env,
            &contract_id,
            &oracle,
            "configure_fixed_shares",
            (shares.clone(),).into_val(&env),
        );
        assert!(client.try_configure_fixed_shares(&shares).is_err());

        // request_sweep with oracle auth fails.
        let args: Vec<Val> = (admin.clone(), 100_i128).into_val(&env);
        mock_one(&env, &contract_id, &oracle, "request_sweep", args.clone());
        assert!(client.try_request_sweep(&admin, &100).is_err());

        // execute_sweep with oracle auth fails.
        mock_one(&env, &contract_id, &oracle, "execute_sweep", vec![&env]);
        assert!(client.try_execute_sweep().is_err());
    }

    #[test]
    fn credit_claim_requires_the_contributor() {
        let (env, client, admin, oracle) = auth_env();
        let contract_id = client.address.clone();
        let root = BytesN::from_array(&env, &[0x44; 32]);
        mock_one(
            &env,
            &contract_id,
            &oracle,
            "post_cycle_root",
            (1_u64, root.clone(), 100_i128).into_val(&env),
        );
        client.post_cycle_root(&1, &root, &100);
        env.ledger().set_timestamp(CHALLENGE_WINDOW_SECS);

        let proof = vec![&env];
        // Oracle's auth cannot authorize a claim on the admin's behalf.
        mock_one(
            &env,
            &contract_id,
            &oracle,
            "credit_claim",
            (admin.clone(), 1_u64, 100_i128, proof.clone()).into_val(&env),
        );
        assert!(client
            .try_credit_claim(&admin, &1, &100, &proof)
            .is_err());

        // The contributor's own auth passes, and the empty proof then fails
        // verification — proving auth is checked first.
        mock_one(
            &env,
            &contract_id,
            &admin,
            "credit_claim",
            (admin.clone(), 1_u64, 100_i128, proof.clone()).into_val(&env),
        );
        assert_eq!(
            client.try_credit_claim(&admin, &1, &100, &proof),
            Err(Ok(SplitStreamError::InvalidProof))
        );
    }
}