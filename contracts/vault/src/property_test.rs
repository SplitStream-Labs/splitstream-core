//! Property-based tests for the two pieces of arithmetic the protocol trusts
//! most: Merkle proof verification and linear vesting.
//!
//! These are deterministic and dependency-free — a seeded xorshift generator
//! produces the cases, so a failure is reproducible from the seed printed in the
//! assertion message. They complement the example-based tests in `test.rs`:
//! those pin exact behaviour at specific boundaries, these sweep many random
//! shapes looking for an input class nobody thought to write down.

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, BytesN, Env, Vec,
};

use crate::merkle::{leaf_hash, verify_proof};
use crate::test::build_manifest;
use crate::{SplitStreamVault, SplitStreamVaultClient};

/// Small deterministic PRNG (xorshift64). Not cryptographic — it only needs to
/// spread test inputs around reproducibly.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // A zero state is a fixed point for xorshift, so never allow it.
        Rng(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Uniform in `[lo, hi]`; panics if the range is empty.
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        assert!(lo <= hi);
        lo + self.next_u64() % (hi - lo + 1)
    }

    fn i128_range(&mut self, lo: i128, hi: i128) -> i128 {
        assert!(lo <= hi);
        let span = (hi - lo) as u128;
        lo + (u128::from(self.next_u64()) % (span + 1)) as i128
    }

    fn bytes32(&mut self, env: &Env) -> BytesN<32> {
        let mut out = [0u8; 32];
        for chunk in out.chunks_mut(8) {
            chunk.copy_from_slice(&self.next_u64().to_be_bytes());
        }
        BytesN::from_array(env, &out)
    }
}

/// Every leaf of a randomly shaped manifest must verify, and any perturbation
/// of the leaf or the proof must be rejected.
#[test]
fn merkle_proofs_hold_for_random_manifests() {
    let env = Env::default();
    let mut rng = Rng::new(0xC0FF_EE00_1234_5678);

    for case in 0..48u64 {
        // 1, 2, 4, 8, 16 or 32 leaves — the tree builder needs a power of two.
        let n = 1usize << rng.range(0, 5);
        let mut entries: std::vec::Vec<(Address, i128)> = std::vec::Vec::with_capacity(n);
        for _ in 0..n {
            entries.push((Address::generate(&env), rng.i128_range(1, 1_000_000_000)));
        }

        let (root, claims) = build_manifest(&env, &entries);
        assert_eq!(claims.len(), n, "case {case}");

        for (addr, amount, proof) in &claims {
            let leaf = leaf_hash(&env, addr, *amount);
            assert!(
                verify_proof(&env, &leaf, proof, &root),
                "case {case}: valid proof rejected for {addr:?}"
            );

            // A different amount must not verify against the same proof.
            let tampered = leaf_hash(&env, addr, amount + 1);
            assert!(
                !verify_proof(&env, &tampered, proof, &root),
                "case {case}: tampered amount accepted for {addr:?}"
            );

            // A proof whose sibling is replaced must not verify. Regenerate
            // until the substitute actually differs, since a 32-byte collision
            // would make the assertion meaningful in the wrong direction.
            if !proof.is_empty() {
                let mut bad: Vec<BytesN<32>> = Vec::new(&env);
                for (i, sibling) in proof.iter().enumerate() {
                    if i == 0 {
                        let mut replacement = rng.bytes32(&env);
                        while replacement == sibling {
                            replacement = rng.bytes32(&env);
                        }
                        bad.push_back(replacement);
                    } else {
                        bad.push_back(sibling);
                    }
                }
                assert!(
                    !verify_proof(&env, &leaf, &bad, &root),
                    "case {case}: tampered proof accepted for {addr:?}"
                );
            } else {
                // A single-leaf tree's root is the leaf itself.
                assert_eq!(root, leaf, "case {case}: single leaf root mismatch");
            }
        }
    }
}

/// A contributor's address must be bound to their amount: the leaf for one
/// contributor must not verify with another contributor's proof.
#[test]
fn merkle_proofs_do_not_transfer_between_contributors() {
    let env = Env::default();
    let mut rng = Rng::new(0xDEAD_BEEF_0BAD_F00D);

    for case in 0..24u64 {
        let n = 1usize << rng.range(1, 4); // at least two leaves
        let mut entries: std::vec::Vec<(Address, i128)> = std::vec::Vec::with_capacity(n);
        for _ in 0..n {
            entries.push((Address::generate(&env), rng.i128_range(1, 10_000)));
        }
        let (root, claims) = build_manifest(&env, &entries);

        let (addr_a, amount_a, _) = &claims[0];
        let (_, _, proof_b) = &claims[1];
        let leaf_a = leaf_hash(&env, addr_a, *amount_a);
        assert!(
            !verify_proof(&env, &leaf_a, proof_b, &root),
            "case {case}: another contributor's proof accepted"
        );
    }
}

/// Vesting must release monotonically, never exceed the schedule total, and
/// always follow `vested = floor(total * elapsed / duration)`.
#[test]
fn vesting_claims_follow_the_linear_schedule() {
    let mut rng = Rng::new(0x5EED_1234_ABCD_0001);

    for case in 0..24u64 {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let oracle = Address::generate(&env);
        let token = env.register_stellar_asset_contract_v2(admin.clone());
        let sac = token::StellarAssetClient::new(&env, &token.address());
        let vault_id = env.register(SplitStreamVault, ());
        let client = SplitStreamVaultClient::new(&env, &vault_id);
        client.initialize(&admin, &oracle, &token.address());

        let total = rng.i128_range(1, 1_000_000);
        let duration = rng.range(1, 400) as u32;
        let start: u32 = rng.range(1, 5_000) as u32;

        sac.mint(&admin, &total);
        client.deposit(&admin, &total);
        env.ledger().set_sequence_number(start);
        client.create_vesting(&admin, &total, &duration);

        let mut claimed_total: i128 = 0;

        // Walk a random, non-decreasing path through the schedule.
        let mut ledger = start;
        for step in 0..6 {
            ledger += rng.range(0, u64::from(duration) / 5 + 1) as u32;
            env.ledger().set_sequence_number(ledger);

            let claimed_now = client.claim_vested(&admin);
            assert!(claimed_now >= 0, "case {case} step {step}: negative claim");
            claimed_total += claimed_now;

            let elapsed = ledger.saturating_sub(start).min(duration);
            let expected_vested = total * i128::from(elapsed) / i128::from(duration);
            assert_eq!(
                claimed_total, expected_vested,
                "case {case} step {step}: cumulative claim diverged from the schedule \
                 (total={total}, duration={duration}, start={start}, ledger={ledger})"
            );

            let stored = client.get_vesting(&admin).expect("schedule exists");
            assert_eq!(stored.claimed, claimed_total, "case {case}: stored claimed");
            assert_eq!(
                client.get_vesting_reserved(),
                total - claimed_total,
                "case {case}: vesting liability drifted"
            );
        }

        // Jump past the end: the remainder must release exactly once.
        env.ledger().set_sequence_number(start + duration);
        let final_claim = client.claim_vested(&admin);
        claimed_total += final_claim;
        assert_eq!(
            claimed_total, total,
            "case {case}: full schedule not released"
        );
        assert_eq!(client.get_vesting_reserved(), 0);
        assert_eq!(
            client.claim_vested(&admin),
            0,
            "case {case}: double release"
        );
    }
}

/// Re-creating a schedule must never reduce the outstanding liability below
/// zero, and the vault's accounting identity must hold throughout:
/// `balance(vault) == available_funds + credited_total + vesting_reserved`.
#[test]
fn vesting_recreation_and_vault_accounting_stay_consistent() {
    let mut rng = Rng::new(0xABCD_0000_1111_2222);

    for case in 0..24u64 {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let oracle = Address::generate(&env);
        let token = env.register_stellar_asset_contract_v2(admin.clone());
        let sac = token::StellarAssetClient::new(&env, &token.address());
        let token_client = token::TokenClient::new(&env, &token.address());
        let vault_id = env.register(SplitStreamVault, ());
        let client = SplitStreamVaultClient::new(&env, &vault_id);
        client.initialize(&admin, &oracle, &token.address());

        let deposit = rng.i128_range(10_000, 1_000_000);
        sac.mint(&admin, &deposit);
        client.deposit(&admin, &deposit);

        env.ledger().set_sequence_number(1_000);
        let mut total = rng.i128_range(1, deposit);
        client.create_vesting(&admin, &total, &200);

        for step in 0..4u64 {
            // Advance, then sometimes claim.
            env.ledger()
                .set_sequence_number(1_000 + (step as u32 + 1) * 60);
            if rng.range(0, 1) == 1 {
                client.claim_vested(&admin);
            }

            let schedule = client.get_vesting(&admin).expect("schedule exists");
            assert!(schedule.claimed >= 0, "case {case}: negative claimed");
            assert!(
                schedule.claimed <= schedule.total,
                "case {case} step {step}: claimed {} exceeds total {}",
                schedule.claimed,
                schedule.total
            );

            // Re-create at or above the already-claimed amount. The zero-growth
            // case exercises the `total == claimed` boundary.
            let claimed = schedule.claimed;
            let headroom = deposit - claimed;
            let growth = if headroom == 0 {
                0
            } else {
                rng.i128_range(0, headroom)
            };
            total = claimed + growth;
            if total <= 0 {
                total = 1;
            }
            client.create_vesting(&admin, &total, &150);

            let schedule = client.get_vesting(&admin).expect("schedule exists");
            assert_eq!(schedule.total, total);
            assert_eq!(
                client.get_vesting_reserved(),
                total - schedule.claimed,
                "case {case} step {step}: liability != total - claimed"
            );

            // Accounting identity: what the vault holds equals what is free
            // plus everything it owes.
            let on_hand = token_client.balance(&vault_id);
            assert_eq!(
                on_hand,
                client.get_available_funds()
                    + client.get_total_credited()
                    + client.get_vesting_reserved(),
                "case {case} step {step}: vault accounting identity broken"
            );
        }
    }
}
