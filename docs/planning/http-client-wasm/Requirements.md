# http-client WASM Compatibility — Requirements Specification

**Version:** 1.0.0
**Date:** 2026-02-07
**Status:** Draft
**Crate:** `http-client` (HTTP client wrapper for Meilisearch)

---

## 1. Problem Statement

The `http-client` crate wraps `reqwest` (async) and `ureq` (sync) behind IP policy enforcement. Unlike most Meilisearch crates, **this crate already compiles to `wasm32-unknown-unknown`** — the Cargo.toml has `cfg(target_arch = "wasm32")` conditional dependencies, and `cargo build --target wasm32-unknown-unknown -p http-client` succeeds today.

However, the WASM support was added for the crate in isolation. Several gaps remain before it is production-ready as part of a WASM Meilisearch pipeline:

1. The `stream` reqwest feature is missing from the WASM dependency (streaming responses unavailable)
2. The `build_with_policies()` method has different signatures on native vs WASM (API divergence)
3. No WASM-specific tests exist
4. Upstream `getrandom` 0.3.x requires `wasm_js` cfg flag at workspace level
5. `reqwest-eventsource` (a consumer) fails on WASM due to missing `bytes_stream()`

## 2. Crate Anatomy

### Source

7 source files across 3 modules:

| File | LOC | WASM Status |
|------|-----|-------------|
| `src/lib.rs` | 75 | SAFE — `ureq` module gated behind `cfg(not(wasm32))` |
| `src/policy.rs` | 140 | SAFE — pure `std::net` IP parsing, no OS deps |
| `src/reqwest/mod.rs` | 223 | SAFE — native/WASM `impl` blocks with `cfg` gates |
| `src/reqwest/request.rs` | 133 | SAFE — wraps `reqwest::RequestBuilder`, no OS deps |
| `src/reqwest/error.rs` | 53 | SAFE — error enum, no OS deps |
| `src/reqwest/resolver.rs` | 52 | NATIVE-ONLY — uses `hyper-util` DNS, already gated |
| `src/ureq/*.rs` | ~200 | NATIVE-ONLY — entire module gated |

### Dependencies

| Dependency | WASM Config | Status |
|------------|-------------|--------|
| `cidr` | Unconditional | COMPATIBLE — pure Rust IP parsing |
| `reqwest` (native) | `cfg(not(wasm32))` | Features: stream, multipart, rustls-tls-native-roots, json |
| `reqwest` (WASM) | `cfg(wasm32)` | Features: multipart, json (no stream, no TLS) |
| `ureq` | `cfg(not(wasm32))` | EXCLUDED from WASM |
| `hyper-util` | `cfg(not(wasm32))` | EXCLUDED from WASM |
| `tower-service` | `cfg(not(wasm32))` | EXCLUDED from WASM |
| `tokio` (dev) | `cfg(not(wasm32))` | EXCLUDED from WASM |

### Consumers (within Meilisearch workspace)

| Consumer | Usage |
|----------|-------|
| `meilisearch` (binary crate) | Main consumer for webhook, vector API calls |
| `milli` (via embedders) | REST/OpenAI/Ollama embedder HTTP calls |
| `reqwest-eventsource` | SSE streaming (currently fails on WASM) |

## 3. Scope

### In Scope

- Validate and document existing WASM compatibility
- Add `stream` feature to WASM reqwest dependency
- Unify `build_with_policies()` API signature across targets
- Add WASM compilation test to CI
- Fix `getrandom` WASM configuration at workspace level
- Document consumer migration path

### Out of Scope

- Fixing `reqwest-eventsource` WASM issues (separate crate)
- Adding a WASM-specific HTTP backend (e.g., `web-sys fetch` directly)
- Modifying consumer crates to use the WASM API

## 4. User Stories

**US-1:** As a WASM consumer, I want to make HTTP requests using the same `http_client::reqwest::Client` API so that embedder and webhook code works unchanged.

**US-2:** As a developer, I want `build_with_policies()` to have a uniform API so that consumers don't need `cfg` gates for client construction.

**US-3:** As a CI maintainer, I want automated verification that the crate compiles on `wasm32-unknown-unknown` so that WASM support doesn't regress.

## 5. Requirements (EARS Format)

### 5.1 Existing WASM Compatibility (Verification)

**REQ-HC-1:** The crate SHALL compile on `wasm32-unknown-unknown` with `cargo build --target wasm32-unknown-unknown -p http-client`.

**REQ-HC-2:** The `ureq` module SHALL NOT be compiled on `wasm32-unknown-unknown`.

**REQ-HC-3:** The `reqwest::resolver` module SHALL NOT be compiled on `wasm32-unknown-unknown`.

**REQ-HC-4:** The `policy` module SHALL be available on all targets (uses only `std::net` types).

**REQ-HC-5:** `Client`, `ClientBuilder`, `RequestBuilder`, `Error`, and `Result` types from `reqwest` module SHALL be available on WASM.

### 5.2 Streaming Support

**REQ-ST-1:** WHEN `wasm32` target is active THEN reqwest SHALL be configured with the `stream` feature to enable `Response::bytes_stream()`.

**REQ-ST-2:** IF the `stream` feature causes WASM compilation failure THEN it SHALL be omitted and the limitation SHALL be documented.

### 5.3 API Uniformity

**REQ-API-1:** `ClientBuilder::build_with_policies()` SHALL accept the same parameters on native and WASM targets.

**REQ-API-2:** IF redirect policy is not supported on WASM THEN `build_with_policies()` SHALL accept the parameter but ignore it, rather than having a different signature.

**REQ-API-3:** `ClientBuilder::build_with_default_policies()` SHALL remain the recommended cross-target API.

### 5.4 getrandom Configuration

**REQ-GR-1:** The workspace SHALL configure `getrandom` for WASM support so that `uuid` and other crates using random number generation work on `wasm32-unknown-unknown`.

**REQ-GR-2:** For `getrandom` 0.2.x, the `js` feature SHALL be enabled on WASM targets.

**REQ-GR-3:** For `getrandom` 0.3.x, the `wasm_js` cfg flag SHALL be set via `.cargo/config.toml` or workspace configuration.

### 5.5 Testing

**REQ-TS-1:** Existing tests SHALL pass on native with no changes.

**REQ-TS-2:** A CI step SHALL verify `cargo check --target wasm32-unknown-unknown -p http-client`.

**REQ-TS-3:** WASM-specific unit tests SHALL verify `Client` construction and IP policy enforcement.

## 6. Constraints

**C-1:** The crate is already WASM-compatible. Changes must not break existing functionality.

**C-2:** `reqwest` WASM support uses `web-sys` Fetch API under the hood — some features (TLS configuration, custom DNS resolver, proxy) are not available.

**C-3:** The `build_with_policies()` native signature includes `redirect::Policy` which doesn't exist on WASM. API unification must handle this gracefully.

**C-4:** `reqwest-eventsource` is a separate crate that needs its own WASM work; http-client should not block on it.

## 7. Acceptance Criteria

1. `cargo build --target wasm32-unknown-unknown -p http-client` succeeds (already passes)
2. `cargo test -p http-client` passes on native (no regression)
3. `build_with_policies()` API is uniform or `build_with_default_policies()` is documented as the cross-target entry point
4. `getrandom` WASM configuration is documented
5. Streaming support status is documented with clear capability matrix
