# http-client WASM Compatibility — Design Document

**Version:** 1.0.0
**Date:** 2026-02-07
**Status:** Draft
**Implements:** `Requirements.md` v1.0.0

---

## 1. Executive Summary

The `http-client` crate is **already WASM-compatible**. This design documents the existing architecture, identifies minor gaps, and specifies targeted improvements. The total work is approximately 2-3 hours.

## 2. Current Architecture (Already Working)

### 2.1 Conditional Dependency Layout

```toml
# Cargo.toml — already in place
[dependencies]
cidr = "0.3.2"                              # Always: pure Rust

[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
ureq = "3.1.4"                              # Native-only: sync HTTP
hyper-util = "0.1.19"                       # Native-only: DNS resolver
tower-service = "0.3.3"                     # Native-only: tower traits
reqwest = { features = ["stream", "multipart", "rustls-tls-native-roots", "json"] }

[target.'cfg(target_arch = "wasm32")'.dependencies]
reqwest = { features = ["multipart", "json"] }  # WASM: minimal features
```

### 2.2 Module Gating

```
src/
├── lib.rs          ─── pub mod reqwest; (always)
│                       #[cfg(not(wasm32))] pub mod ureq; (native-only)
│                       pub mod policy; (always)
│
├── policy.rs       ─── IpPolicy, is_global_4/6 (pure std::net, no OS deps)
│
├── reqwest/
│   ├── mod.rs      ─── Client, ClientBuilder with cfg'd impl blocks
│   ├── request.rs  ─── RequestBuilder (target-agnostic wrapper)
│   ├── error.rs    ─── Error enum (target-agnostic)
│   └── resolver.rs ─── #[cfg(not(wasm32))] ExternalRequestResolver
│
└── ureq/           ─── Entire module gated behind cfg(not(wasm32))
    ├── mod.rs
    ├── config.rs
    └── resolver.rs
```

### 2.3 ClientBuilder Dual Implementation

The crate already handles the native/WASM API split:

```rust
// Native: full build with DNS resolver + redirect policy
#[cfg(not(target_arch = "wasm32"))]
impl ClientBuilder {
    pub fn build_with_policies(
        self, ip_policy: IpPolicy, redirect_policy: redirect::Policy
    ) -> Result<Client> { ... }
}

// WASM: simplified build (no DNS resolver, no redirect)
#[cfg(target_arch = "wasm32")]
impl ClientBuilder {
    pub fn build_with_policies(self, ip_policy: IpPolicy) -> Result<Client> { ... }
}

// Uniform API across both targets
impl ClientBuilder {
    pub fn build_with_default_policies(self, ip_policy: IpPolicy) -> Result<Client> {
        #[cfg(not(target_arch = "wasm32"))]
        { self.build_with_policies(ip_policy, Default::default()) }
        #[cfg(target_arch = "wasm32")]
        { self.build_with_policies(ip_policy) }
    }
}
```

## 3. Gaps and Improvements

### 3.1 Gap: Missing `stream` Feature on WASM

**Current state:** WASM reqwest has `["multipart", "json"]` — no `stream`.

**Impact:** `Response::bytes_stream()` is unavailable on WASM. The `reqwest-eventsource` crate fails because it calls `bytes_stream()`.

**Decision D-1:** Add `stream` to WASM reqwest features if it compiles. Reqwest's `stream` feature on WASM wraps the ReadableStream API via `web-sys`.

```toml
# Proposed change
[target.'cfg(target_arch = "wasm32")'.dependencies]
reqwest = { version = "0.12.24", default-features = false, features = [
    "stream",      # ADD: enables bytes_stream() via web-sys
    "multipart",
    "json",
] }
```

**Risk:** Low. Reqwest documents `stream` as WASM-compatible since v0.12.

### 3.2 Gap: API Signature Divergence

**Current state:** `build_with_policies()` takes 2 args on native, 1 on WASM.

**Decision D-2:** Keep the divergent signatures but document `build_with_default_policies()` as the canonical cross-target API. Reason: changing the native signature would break all existing callers. The uniform method already exists.

**Consumer guidance:**
```rust
// Cross-target code should use:
let client = ClientBuilder::new()
    .build_with_default_policies(ip_policy)?;

// Native-only code can use the full version:
#[cfg(not(target_arch = "wasm32"))]
let client = ClientBuilder::new()
    .build_with_policies(ip_policy, redirect::Policy::limited(10))?;
```

### 3.3 Gap: getrandom WASM Configuration

**Current state:** Workspace uses `getrandom` 0.2.x (via `uuid`) and 0.3.x (via other deps). Both need WASM configuration.

**Decision D-3:** Configure getrandom at workspace level.

For 0.2.x:
```toml
# Workspace Cargo.toml
[target.'cfg(target_arch = "wasm32")'.dependencies]
getrandom = { version = "0.2", features = ["js"] }
```

For 0.3.x, set cfg flag:
```toml
# .cargo/config.toml
[target.wasm32-unknown-unknown]
rustflags = ['--cfg', 'getrandom_backend="wasm_js"']
```

### 3.4 Gap: No WASM Tests

**Current state:** All tests are gated behind `cfg(not(target_arch = "wasm32"))`.

**Decision D-4:** Add basic WASM smoke tests using `wasm-bindgen-test`:

```rust
#[cfg(target_arch = "wasm32")]
#[cfg(test)]
mod wasm_tests {
    use wasm_bindgen_test::*;
    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    async fn client_construction() {
        let client = crate::reqwest::ClientBuilder::new()
            .build_with_default_policies(
                crate::policy::IpPolicy::danger_always_allow()
            )
            .unwrap();
        // Client constructed successfully on WASM
    }
}
```

## 4. WASM Capability Matrix

| Feature | Native | WASM | Notes |
|---------|--------|------|-------|
| `reqwest::Client` | YES | YES | Wraps web-sys Fetch on WASM |
| `ureq::Agent` | YES | NO | Sync HTTP, requires OS sockets |
| IP Policy enforcement | YES | YES | Pure `std::net` parsing |
| DNS resolver wrapping | YES | NO | Browser handles DNS |
| Redirect policy | YES | NO | Browser handles redirects |
| TLS configuration | YES | NO | Browser handles TLS |
| Proxy configuration | YES | NO | Browser handles proxies |
| Streaming responses | YES | TBD | Depends on `stream` feature test |
| Multipart upload | YES | YES | Via form data API |
| JSON serialization | YES | YES | Pure Rust serde |

## 5. Design Decisions Log

| ID | Decision | Rationale |
|----|----------|-----------|
| D-1 | Add `stream` feature to WASM reqwest | Enables `bytes_stream()` needed by consumers; reqwest supports it on WASM |
| D-2 | Keep divergent `build_with_policies()` signatures | Breaking native callers isn't worth it; `build_with_default_policies()` already provides uniformity |
| D-3 | Configure getrandom at workspace level | Affects all crates using uuid/rand, not http-client specific |
| D-4 | Add wasm-bindgen-test for WASM smoke tests | Prevent regressions; current tests are all native-only |

## 6. Architecture Diagram

```
┌───────────────────────────────────────────────────────┐
│                    http-client                         │
│                                                       │
│  ┌─────────────────────────────────────────────────┐  │
│  │             policy.rs (always)                   │  │
│  │  IpPolicy::check_ip() — pure std::net           │  │
│  └─────────────────────────────────────────────────┘  │
│                        │                              │
│        ┌───────────────┴──────────────┐               │
│        │                              │               │
│  ┌─────▼──────┐              ┌────────▼────────┐      │
│  │  reqwest/   │              │  ureq/ (native) │      │
│  │  (always)   │              │  cfg(not(wasm)) │      │
│  │             │              │                 │      │
│  │ Client      │              │ Agent           │      │
│  │ Builder     │              │ Config          │      │
│  │ Request     │              │ Resolver        │      │
│  │ Error       │              └─────────────────┘      │
│  │             │                                      │
│  │ ┌─────────┐ │                                      │
│  │ │resolver │ │  ◄── cfg(not(wasm32)) only           │
│  │ └─────────┘ │                                      │
│  └─────────────┘                                      │
└───────────────────────────────────────────────────────┘

On native: reqwest → hyper → tokio → OS sockets
On WASM:   reqwest → web-sys → browser Fetch API
```

## 7. Consumer Impact

| Consumer | Impact | Action Required |
|----------|--------|-----------------|
| `meilisearch` binary | None | Already uses `build_with_default_policies()` or native-only paths |
| `milli` embedders | None | HTTP calls via `reqwest::Client` work on WASM |
| `reqwest-eventsource` | Separate work | Needs own WASM adaptation (bytes_stream, event source) |
| `wilysearch` | None | Can use http-client on WASM if needed |
