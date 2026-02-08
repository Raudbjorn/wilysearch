# meilisearch-types WASM Compatibility — Implementation Tasks

**Version:** 1.0.0
**Date:** 2026-02-07
**Status:** Draft
**Implements:** `Design.md` v1.0.0
**Traceability:** Each task references Requirements (REQ-*) and Design decisions (D-*)

---

## Task Sequencing

The crate has a well-implemented `native` feature flag system. Only 3 cross-module reference leaks need fixing. Total effort: **~2 hours**.

Sequence:
1. **Fix leaks** — patch the 3 files with missing cfg gates
2. **Verify** — ensure both native and no-default-features compile
3. **Test** — verify no regressions, add WASM CI
4. **Document** — capture WASM-safe API surface

---

## Epic 1: Cross-Module Reference Fixes

### Task 1.1: Fix `star_or.rs` — Gate `FromQueryParameter` import

**Problem:** Line 11 imports `crate::deserr::query_params::FromQueryParameter`, but `crate::deserr` is `#[cfg(feature = "native")]`.

**Fix:**
- Add `#[cfg(feature = "native")]` to the import on line 11
- Gate the `impl FromQueryParameter for OptionStarOr<T>` block behind `#[cfg(feature = "native")]`
- Gate any `Deserr` impl that has `T: FromQueryParameter` bound behind `#[cfg(feature = "native")]`
- Provide a `#[cfg(not(feature = "native"))]` alternative `Deserr` impl if needed, or just omit (query param parsing is HTTP-specific)

**Files:** `crates/meilisearch-types/src/star_or.rs`
_Requirements: REQ-FIX-1, REQ-FIX-3, Design D-1, D-3_

### Task 1.2: Fix `webhooks.rs` — Remove `crate::settings::hide_secret` dependency

**Problem:** Lines 24 and 26 call `crate::settings::hide_secret(value, N)`, but `crate::settings` is `#[cfg(feature = "native")]`.

**Fix (Option A — Preferred):** Inline the secret hiding logic:
```rust
impl Webhook {
    pub fn redact_authorization_header(&mut self) {
        for value in self.headers.iter_mut()
            .filter_map(|(name, value)| {
                name.eq_ignore_ascii_case("authorization").then_some(value)
            })
        {
            let prefix_len = if value.starts_with("Bearer ") {
                "Bearer ".len()
            } else {
                0
            };
            if value.len() > prefix_len {
                value.replace_range(prefix_len.., &"*".repeat(value.len() - prefix_len));
            }
        }
    }
}
```

**Fix (Option B):** Gate the entire method behind `#[cfg(feature = "native")]`.

**Files:** `crates/meilisearch-types/src/webhooks.rs`
_Requirements: REQ-FIX-2, Design D-2_

### Task 1.3: Verify `document_formats.rs` internal cfg gates

- Read through the 23 internal cfg gates in `document_formats.rs`
- Confirm the WASM-safe type variants (`DocumentPayloadError`) compile without `native`
- Confirm `read_csv`, `read_json`, `read_ndjson` functions are properly gated behind `native`
- No changes expected — verification only

**Files:** `crates/meilisearch-types/src/document_formats.rs`
_Requirements: REQ-TYPE-2, Design D-5_

### Task 1.4: Verify `error.rs` internal cfg gates

- Confirm all `impl ErrorCode for milli::Error`, `impl ErrorCode for file_store::Error`, etc. are gated
- Confirm core `ResponseError`, `Code`, `ErrorCode` types compile without `native`
- No changes expected — verification only

**Files:** `crates/meilisearch-types/src/error.rs`
_Requirements: REQ-TYPE-1, Design D-5_

---

## Epic 2: Compilation Verification

### Task 2.1: Verify `--no-default-features` compilation
- Run `cargo check --no-default-features -p meilisearch-types`
- Must succeed with 0 errors (currently fails with 6)
- **Files:** None (verification only)
- _Requirements: REQ-MT-1_

### Task 2.2: Verify WASM compilation
- Run `cargo check --target wasm32-unknown-unknown --no-default-features -p meilisearch-types`
- Must succeed
- If `getrandom` errors occur, configure at workspace level (see Task 3.1)
- **Files:** None (verification only)
- _Requirements: REQ-MT-2_

### Task 2.3: Verify native compilation (no regression)
- Run `cargo check --features native -p meilisearch-types`
- Must succeed unchanged
- **Files:** None (verification only)
- _Requirements: REQ-MT-3, REQ-BC-1_

### Task 2.4: Verify workspace compilation
- Run `cargo check` for full workspace
- Ensure no consumer crate is broken by the cfg changes
- **Files:** None (verification only)
- _Requirements: REQ-BC-1, REQ-BC-2_

---

## Epic 3: getrandom Configuration

### Task 3.1: Configure getrandom for WASM
- Same as http-client Task 4.1/4.2 — workspace-level configuration
- Add `getrandom` `js` feature for 0.2.x on WASM targets
- Configure `wasm_js` cfg flag for 0.3.x
- **Files:** Workspace `Cargo.toml`, `.cargo/config.toml`
- _Requirements: REQ-GR-1_

---

## Epic 4: Testing & CI

### Task 4.1: Run native tests
- Run `cargo test --features native -p meilisearch-types`
- All existing tests must pass
- **Files:** None (verification only)
- _Requirements: REQ-TS-1_

### Task 4.2: Add CI WASM check
- Add `cargo check --target wasm32-unknown-unknown --no-default-features -p meilisearch-types` to CI
- **Files:** CI configuration
- _Requirements: REQ-TS-2_

---

## Epic 5: Documentation

### Task 5.1: Document WASM-safe API surface
- Add crate-level documentation listing which modules/types are available without `native`
- Document the `native` feature flag purpose
- **Files:** `crates/meilisearch-types/src/lib.rs` (doc comments)

---

## Dependency Graph

```
1.1 (fix star_or.rs)     ──┐
1.2 (fix webhooks.rs)    ──┤
1.3 (verify doc_formats) ──┤
1.4 (verify error.rs)    ──┘
                            │
                            ▼
              2.1 (check --no-default-features)
              2.2 (check wasm32 target) ◄── 3.1 (getrandom cfg)
              2.3 (check native)
              2.4 (check workspace)
                            │
                            ▼
              4.1 (native tests)
              4.2 (CI WASM check)
                            │
                            ▼
              5.1 (documentation)
```

Tasks 1.1 and 1.2 are the only code changes. Everything else is verification or documentation.

---

## Effort Estimates

| Epic | Tasks | Estimated Effort |
|------|-------|-----------------|
| **1. Fixes** | 1.1–1.4 | 45 min |
| **2. Verification** | 2.1–2.4 | 20 min |
| **3. getrandom** | 3.1 | 15 min |
| **4. Testing & CI** | 4.1–4.2 | 15 min |
| **5. Documentation** | 5.1 | 15 min |
| **Total** | **11 tasks** | **~2 hours** |

---

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Inlining `hide_secret` diverges from upstream | Low | Low | Verify upstream `hide_secret` implementation matches |
| `star_or.rs` cfg changes break native Deserr impls | Low | Medium | Test with `--features native` after changes |
| `document_formats.rs` has untested WASM paths | Medium | Low | The dual code paths exist but may not be exercised; add basic tests |
| getrandom 0.3.x cfg breaks workspace | Low | High | Test both native and WASM targets after workspace config |

---

## Current WASM Readiness Checklist

- [x] `native` feature flag defined in Cargo.toml
- [x] Native-only dependencies gated behind `native` feature
- [x] 14 native-only modules gated in lib.rs
- [x] `document_formats.rs` has dual native/WASM code paths (23 cfg gates)
- [x] `error.rs` has internal cfg gates for native types (11 cfg gates)
- [x] `features.rs`, `index_uid.rs`, `network.rs` are WASM-clean
- [ ] `star_or.rs` — `FromQueryParameter` import needs cfg gate
- [ ] `webhooks.rs` — `hide_secret` cross-module ref needs fix
- [ ] `--no-default-features` compilation succeeds
- [ ] `wasm32-unknown-unknown` compilation succeeds
- [ ] getrandom configured for WASM at workspace level
- [ ] CI verifies WASM compilation
- [ ] WASM-safe API surface documented

---

## Post-Fix Module Status

After the 2 code fixes (Tasks 1.1 and 1.2), the crate status:

| Module | native | no-default-features | Notes |
|--------|--------|---------------------|-------|
| `archive_ext` | COMPILED | EXCLUDED | fs operations |
| `batch_view` | COMPILED | EXCLUDED | milli types |
| `batches` | COMPILED | EXCLUDED | milli types |
| `compression` | COMPILED | EXCLUDED | fs + tar |
| `deserr` | COMPILED | EXCLUDED | actix-web |
| `document_formats` | FULL | TYPES ONLY | Functions gated, types available |
| `error` | FULL | CORE ONLY | ErrorCode impls for milli gated |
| `features` | COMPILED | COMPILED | Pure serde |
| `index_uid` | COMPILED | COMPILED | Validation + serde |
| `index_uid_pattern` | COMPILED | COMPILED | Pattern matching |
| `keys` | COMPILED | EXCLUDED | milli types |
| `locales` | COMPILED | EXCLUDED | milli types |
| `network` | COMPILED | COMPILED | Pure serde |
| `settings` | COMPILED | EXCLUDED | Heavy milli usage |
| `star_or` | FULL | CORE ONLY | Query param integration gated |
| `task_view` | COMPILED | EXCLUDED | milli types |
| `tasks` | COMPILED | EXCLUDED | milli types |
| `versioning` | COMPILED | EXCLUDED | fs + tempfile + heed |
| `webhooks` | COMPILED | COMPILED | hide_secret inlined |
