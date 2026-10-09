## Description
<!-- Provide a brief description of the changes in this pull request. -->

## Non-negotiables checklist
<!-- Ensure all applicable items from CONTRIBUTING.md are satisfied. -->
- [ ] Merkle leaf hashing is byte-identical to the frozen format (`sha256(contributor.to_xdr(env) ++ amount.to_xdr(env))`), or the change is coordinated with splitstream-actions and the golden fixtures.
- [ ] No floats anywhere — all monetary math is integer basis-points.
- [ ] No `unwrap()` / `expect()` outside `#[cfg(test)]` code.
- [ ] Every state-changing function emits an event.
- [ ] Every persistent-storage write extends TTL in the same call (`storage::bump_persistent`); every admin-gated call bumps the instance TTL (`storage::bump_instance`).
- [ ] Every `require_auth()` targets the correct role (admin vs oracle vs contributor).
- [ ] Error discriminants are unchanged (they are on-chain ABI).
- [ ] No new unbounded loops over the contributor set (pull-payment preserved).

## Verification
<!-- Describe the tests run and any manual verification. -->
- [ ] `cargo test --workspace` passes cleanly
- [ ] `cargo clippy --all-targets -- -D warnings` passes cleanly
- [ ] `stellar contract build --package splitstream-vault` succeeds
- [ ] Details / notes:
