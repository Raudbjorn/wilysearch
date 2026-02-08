# http-client WASM Compatibility — Implementation Tasks

**Version:** 1.0.0
**Date:** 2026-02-07
**Status:** Draft
**Implements:** `Design.md` v1.0.0
**Traceability:** Each task references Requirements (REQ-*) and Design decisions (D-*)

---

## Task Sequencing

This crate is already WASM-compatible. The work is validation, minor improvements, and documentation. Total effort: **2-3 hours**.

---

## Epic 1: Validation (Already Working)

### Task 1.1: Verify WASM compilation
- Run `cargo build --target wasm32-unknown-unknown -p http-client`
- Confirm it succeeds (it does today)
- Document the output
- **Files:** None (verification only)
- _Requirements: REQ-HC-1_

### Task 1.2: Verify module gating
- Confirm `ureq` module is excluded on WASM (`cfg(not(wasm32))`)
- Confirm `reqwest/resolver.rs` is excluded on WASM (`cfg(not(wasm32))`)
- Confirm `policy.rs` compiles on both targets
- **Files:** None (verification only)
- _Requirements: REQ-HC-2, REQ-HC-3, REQ-HC-4, REQ-HC-5_

---

## Epic 2: Streaming Feature

### Task 2.1: Add `stream` feature to WASM reqwest
- Add `"stream"` to the WASM reqwest features list in Cargo.toml
- Run `cargo build --target wasm32-unknown-unknown -p http-client`
- If it compiles: keep the change
- If it fails: revert and document the limitation
- **Files:** `crates/http-client/Cargo.toml`
- _Requirements: REQ-ST-1, REQ-ST-2, Design D-1_

```toml
# Proposed
[target.'cfg(target_arch = "wasm32")'.dependencies]
reqwest = { version = "0.12.24", default-features = false, features = [
    "stream",
    "multipart",
    "json",
] }
```

---

## Epic 3: API Documentation

### Task 3.1: Document cross-target API guidance
- Add doc comments to `build_with_default_policies()` marking it as the recommended cross-target API
- Add `#[doc]` note on native `build_with_policies()` explaining the WASM divergence
- **Files:** `crates/http-client/src/reqwest/mod.rs`
- _Requirements: REQ-API-1, REQ-API-2, REQ-API-3, Design D-2_

### Task 3.2: Document WASM capability matrix
- Add a `# WASM Support` section to the crate-level documentation (lib.rs doc comment)
- List what works, what doesn't, and why
- **Files:** `crates/http-client/src/lib.rs`
- _Requirements: REQ-HC-1_

---

## Epic 4: getrandom Configuration

### Task 4.1: Configure getrandom 0.2.x for WASM
- Add `getrandom` with `js` feature to workspace WASM dependencies
- **Files:** Workspace `Cargo.toml`
- _Requirements: REQ-GR-1, REQ-GR-2, Design D-3_

### Task 4.2: Configure getrandom 0.3.x for WASM
- Add `--cfg getrandom_backend="wasm_js"` to `.cargo/config.toml` for `wasm32-unknown-unknown`
- Verify `cargo check --target wasm32-unknown-unknown` passes for workspace crates
- **Files:** `.cargo/config.toml`
- _Requirements: REQ-GR-1, REQ-GR-3, Design D-3_

---

## Epic 5: Testing

### Task 5.1: Add wasm-bindgen-test dev-dependency
- Add `wasm-bindgen-test` to WASM dev-dependencies
- **Files:** `crates/http-client/Cargo.toml`
- _Requirements: REQ-TS-3, Design D-4_

```toml
[target.'cfg(target_arch = "wasm32")'.dev-dependencies]
wasm-bindgen-test = "0.3"
```

### Task 5.2: Write WASM smoke tests
- Test `ClientBuilder::new().build_with_default_policies()` succeeds
- Test `IpPolicy` creation and IP checking
- Test `RequestBuilder` construction
- **Files:** `crates/http-client/src/lib.rs` (WASM test module)
- _Requirements: REQ-TS-3, Design D-4_

### Task 5.3: Verify native tests pass
- Run `cargo test -p http-client`
- Ensure no regressions from any changes
- **Files:** None (verification only)
- _Requirements: REQ-TS-1_

### Task 5.4: Add CI WASM check
- Add `cargo check --target wasm32-unknown-unknown -p http-client` to CI pipeline
- **Files:** CI configuration (`.github/workflows/`)
- _Requirements: REQ-TS-2_

---

## Dependency Graph

```
1.1 (verify compilation) ──┐
1.2 (verify gating)        │
                            ├── 2.1 (add stream feature)
                            │    │
                            ├── 3.1 (API docs)
                            ├── 3.2 (WASM capability docs)
                            │
                            ├── 4.1 (getrandom 0.2.x)
                            ├── 4.2 (getrandom 0.3.x)
                            │    │
                            └── 5.1 (wasm-bindgen-test dep)
                                 │
                                 ├── 5.2 (WASM smoke tests)
                                 ├── 5.3 (native test verification)
                                 └── 5.4 (CI check)
```

All epics can proceed **in parallel** since the crate already compiles.

---

## Effort Estimates

| Epic | Tasks | Estimated Effort |
|------|-------|-----------------|
| **1. Validation** | 1.1–1.2 | 15 min |
| **2. Streaming** | 2.1 | 15 min |
| **3. Documentation** | 3.1–3.2 | 30 min |
| **4. getrandom** | 4.1–4.2 | 30 min |
| **5. Testing** | 5.1–5.4 | 45 min |
| **Total** | **10 tasks** | **~2.5 hours** |

---

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| `stream` feature fails on WASM | Low | Low | Revert and document; not critical for initial WASM |
| getrandom 0.3.x cfg breaks native | Low | High | Test both targets after config change |
| reqwest WASM API changes | Very Low | Medium | Pin reqwest version |
| wasm-bindgen-test flaky in CI | Low | Low | Allow retries; tests are simple construction checks |

---

## Current WASM Readiness Checklist

- [x] Cargo.toml has cfg-gated dependencies
- [x] `ureq` module excluded on WASM
- [x] `reqwest/resolver.rs` excluded on WASM
- [x] `policy.rs` is target-agnostic
- [x] `Client`/`ClientBuilder` have WASM impl blocks
- [x] `build_with_default_policies()` works cross-target
- [x] `cargo build --target wasm32-unknown-unknown` passes
- [ ] `stream` feature enabled on WASM
- [ ] getrandom configured for WASM at workspace level
- [ ] WASM-specific tests exist
- [ ] CI verifies WASM compilation
