# meilisearch-types WASM Compatibility — Requirements Specification

**Version:** 1.0.0
**Date:** 2026-02-07
**Status:** Draft
**Crate:** `meilisearch-types` (shared types for the Meilisearch ecosystem)

---

## 1. Problem Statement

The `meilisearch-types` crate is the shared type library for the Meilisearch ecosystem — error types, settings structures, task definitions, document format parsing, and serialization. It has a **`native` feature flag already in place** that gates most WASM-incompatible modules behind `#[cfg(feature = "native")]`. However, compiling with `--no-default-features` (disabling `native`) currently fails with **6 errors** due to three cross-module reference leaks:

1. `star_or.rs` imports from the native-gated `crate::deserr` module
2. `webhooks.rs` calls `crate::settings::hide_secret()` from the native-gated `settings` module
3. `star_or.rs` has trait bound issues related to the missing `FromQueryParameter` trait

The crate is approximately 90% WASM-ready. The remaining work is fixing these three leaks and adding WASM compilation validation.

## 2. Crate Anatomy

### Source

~35 source files across multiple modules.

### Module Classification (as declared in `lib.rs`)

**Native-only modules** (already gated with `#[cfg(feature = "native")]`):

| Module | Reason for Native Dependency |
|--------|------------------------------|
| `archive_ext` | `std::fs::DirEntry`, `tar` unpacking with filesystem |
| `batch_view` | `milli::progress::ProgressView` |
| `batches` | `milli::progress` types |
| `community_edition` | `milli::update::new::indexer` |
| `compression` | `std::fs::{File, create_dir_all}`, `tar`, `flate2` |
| `deserr` | `actix-web` integration |
| `enterprise_edition` | `milli::update::new::indexer` |
| `facet_values_sort` | `milli::OrderBy` |
| `keys` | `milli::update::Setting` |
| `locales` | `milli::LocalizedAttributesRule` |
| `settings` | Heavy `milli` usage (15+ milli imports) |
| `task_view` | `milli::Object` |
| `tasks` | `milli::update` types |
| `versioning` | `std::fs`, `tempfile`, `milli::heed` |

**WASM-safe modules** (no cfg gate, available on all targets):

| Module | Dependencies | Status |
|--------|-------------|--------|
| `document_formats` | Internal cfg gates for native/WASM variants | HAS INTERNAL CFG |
| `error` | Internal cfg gates for native types (actix, milli, heed) | HAS INTERNAL CFG |
| `features` | Pure serde types | CLEAN |
| `index_uid` | `deserr`, `serde`, `utoipa` | CLEAN |
| `index_uid_pattern` | `serde` | CLEAN |
| `network` | `serde`, `uuid` | CLEAN |
| `star_or` | `deserr`, `serde`, `utoipa` — **LEAKS** `crate::deserr` import | BROKEN |
| `webhooks` | `serde`, `uuid` — **LEAKS** `crate::settings::hide_secret` | BROKEN |

### Dependencies

| Dependency | Feature Gate | WASM Status |
|------------|-------------|-------------|
| `actix-web` | `native` | INCOMPATIBLE — native web framework |
| `file-store` | `native` | INCOMPATIBLE — filesystem I/O |
| `memmap2` | `native` | INCOMPATIBLE — OS mmap |
| `milli` | `native` | INCOMPATIBLE — LMDB, rayon, etc. |
| `tar` | `native` | COMPATIBLE but filesystem-oriented |
| `tempfile` | `native` | INCOMPATIBLE — OS temp files |
| `tokio` | `native` | PARTIAL — WASM features only |
| `anyhow` | Always | COMPATIBLE |
| `serde` / `serde_json` | Always | COMPATIBLE |
| `deserr` | Always (but `deserr/actix-web` feature is `native`) | COMPATIBLE (core) |
| `uuid` | Always | COMPATIBLE (needs `getrandom` cfg for WASM) |
| `utoipa` | Always | COMPATIBLE |
| `csv` | Always | COMPATIBLE |
| `roaring` | Always | COMPATIBLE |
| `time` | Always | COMPATIBLE |

### Consumers (5 workspace crates)

| Consumer | Usage |
|----------|-------|
| `dump` | Task types, settings, document formats |
| `index-scheduler` | Full usage — tasks, batches, settings, file-store |
| `meilisearch-auth` | Keys, error types |
| `meilisearch` | Everything — routes use settings, tasks, document formats |
| `meilitool` | Versioning, error types |

### External consumer

| Consumer | Usage |
|----------|-------|
| `wilysearch` | Settings types, error types, document formats (via `--no-default-features` possible) |

## 3. Scope

### In Scope

- Fix 3 cross-module reference leaks so `--no-default-features` compiles
- Verify WASM compilation with `cargo check --target wasm32-unknown-unknown --no-default-features`
- Add `wasm` feature flag for explicit opt-in (currently just absence of `native`)
- Configure `getrandom` for WASM at workspace level
- Document which types/modules are available without `native`

### Out of Scope

- Modifying native-gated modules (they stay native-only)
- Making `milli` types available on WASM (that's the milli-wasm spec)
- Replacing `actix-web` error integration for WASM

## 4. User Stories

**US-1:** As a `wilysearch` developer, I want `meilisearch-types` to compile with `--no-default-features` so I can use shared types (error codes, document formats, index UIDs) without pulling in milli and actix-web.

**US-2:** As a WASM application developer, I want to parse document payloads (JSON, CSV, NDJSON) using the same format types as Meilisearch so my client-side processing matches server expectations.

**US-3:** As a Meilisearch maintainer, I want the native build to remain completely unchanged so there are zero regressions.

## 5. Requirements (EARS Format)

### 5.1 Compilation

**REQ-MT-1:** The crate SHALL compile with `cargo check --no-default-features -p meilisearch-types` (no errors).

**REQ-MT-2:** The crate SHALL compile with `cargo check --target wasm32-unknown-unknown --no-default-features -p meilisearch-types`.

**REQ-MT-3:** The crate SHALL compile with `cargo check --features native -p meilisearch-types` with zero regressions.

### 5.2 Cross-Module Reference Fixes

**REQ-FIX-1:** `star_or.rs` SHALL NOT import from `crate::deserr` when `native` feature is disabled. The `FromQueryParameter` integration SHALL be gated behind `#[cfg(feature = "native")]`.

**REQ-FIX-2:** `webhooks.rs` SHALL NOT call `crate::settings::hide_secret()` when `native` feature is disabled. A WASM-safe fallback or cfg gate SHALL be provided.

**REQ-FIX-3:** `star_or.rs` trait bounds involving `FromQueryParameter` SHALL be gated behind `#[cfg(feature = "native")]`.

### 5.3 Feature Flags

**REQ-FF-1:** The existing `native` (default) feature flag SHALL remain unchanged.

**REQ-FF-2:** A `wasm` feature flag MAY be added for explicit WASM opt-in, enabling getrandom WASM support and any WASM-specific configuration.

**REQ-FF-3:** WHEN `native` is disabled THEN all modules listed as "WASM-safe" in lib.rs SHALL compile successfully.

### 5.4 Available Types on WASM

**REQ-TYPE-1:** The following types SHALL be available without `native`:
- `ResponseError`, `Code`, `ErrorCode` (from `error`)
- `PayloadType`, `DocumentFormatError` (from `document_formats`)
- `RuntimeTogglableFeatures` (from `features`)
- `IndexUid`, `IndexUidFormatError` (from `index_uid`)
- `IndexUidPattern` (from `index_uid_pattern`)
- `Network`, `Remote` (from `network`)
- `StarOr`, `OptionStarOr` (from `star_or`)
- `Webhook`, `WebhooksView` (from `webhooks`)
- `Document`, `InstanceUid` (from `lib.rs`)

**REQ-TYPE-2:** The `document_formats` module SHALL provide WASM-safe error types (`DocumentPayloadError`) when `native` is disabled.

### 5.5 getrandom Configuration

**REQ-GR-1:** The workspace SHALL configure `getrandom` for WASM support so that `uuid` v4 generation works on `wasm32-unknown-unknown`.

### 5.6 Backward Compatibility

**REQ-BC-1:** All existing consumer crates SHALL compile unchanged with `--features native`.

**REQ-BC-2:** The public API surface SHALL not change when `native` is enabled.

**REQ-BC-3:** No new dependencies SHALL be added to the always-on (non-optional) dependency list.

### 5.7 Testing

**REQ-TS-1:** All existing tests SHALL pass with `--features native`.

**REQ-TS-2:** A CI step SHALL verify `cargo check --target wasm32-unknown-unknown --no-default-features -p meilisearch-types`.

## 6. Constraints

**C-1:** The `deserr` crate's `actix-web` feature is gated behind `native` in Cargo.toml — the core `deserr` crate is WASM-compatible but `deserr/actix-web` is not.

**C-2:** `document_formats.rs` has dual code paths (native with mmap + milli types, non-native with simplified error types). These are already implemented and should not be changed.

**C-3:** `error.rs` has extensive internal cfg gating. The ErrorCode trait and Code enum are WASM-safe; the `impl ErrorCode for milli::Error` blocks are native-only.

**C-4:** The `star_or.rs` `FromQueryParameter` integration is only meaningful for actix-web HTTP query parsing — it has no use case on WASM.

## 7. Acceptance Criteria

1. `cargo check --no-default-features -p meilisearch-types` succeeds (0 errors, currently 6)
2. `cargo check --target wasm32-unknown-unknown --no-default-features -p meilisearch-types` succeeds
3. `cargo check --features native -p meilisearch-types` succeeds (no regression)
4. `cargo test --features native -p meilisearch-types` passes
5. All workspace crates compile unchanged with `--features native`
