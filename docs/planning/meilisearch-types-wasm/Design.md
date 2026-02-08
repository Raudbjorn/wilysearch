# meilisearch-types WASM Compatibility — Design Document

**Version:** 1.0.0
**Date:** 2026-02-07
**Status:** Draft
**Implements:** `Requirements.md` v1.0.0

---

## 1. Executive Summary

The `meilisearch-types` crate is **90% WASM-ready**. The upstream Meilisearch team has already implemented a comprehensive `native` feature flag system that gates all modules depending on `milli`, `actix-web`, `memmap2`, `tempfile`, and `file-store`. Two modules with internal dual code paths (`document_formats.rs`, `error.rs`) provide WASM-safe alternatives when `native` is disabled.

Only **3 cross-module reference leaks** prevent `--no-default-features` compilation. This design specifies targeted fixes totaling ~2 hours of work.

## 2. Current Architecture

### 2.1 Feature Flag System (Already Implemented)

```toml
# Cargo.toml — already in place
[features]
default = ["native"]
native = [
    "dep:milli",           # Core indexing engine
    "dep:actix-web",       # Web framework types
    "dep:memmap2",         # Memory-mapped file I/O
    "dep:tempfile",        # Temporary file creation
    "dep:file-store",      # Update file storage
    "dep:tokio",           # Async runtime
    "dep:tar",             # Archive handling
    "deserr/actix-web",    # Deserr web integration
]
```

### 2.2 Module Gating Pattern (Already Implemented)

```rust
// lib.rs — already in place
// Native-only modules (14 modules gated)
#[cfg(feature = "native")] pub mod archive_ext;
#[cfg(feature = "native")] pub mod batch_view;
#[cfg(feature = "native")] pub mod batches;
#[cfg(feature = "native")] pub mod compression;
#[cfg(feature = "native")] pub mod deserr;
#[cfg(feature = "native")] pub mod settings;
#[cfg(feature = "native")] pub mod versioning;
// ... etc

// WASM-safe modules (8 modules, always compiled)
pub mod document_formats;
pub mod error;
pub mod features;
pub mod index_uid;
pub mod index_uid_pattern;
pub mod network;
pub mod star_or;
pub mod webhooks;
```

### 2.3 Internal Dual Code Paths (Already Implemented)

**`document_formats.rs`** — 23 internal cfg gates:
```rust
#[cfg(feature = "native")]
use memmap2::Mmap;
#[cfg(feature = "native")]
use milli::documents::Error;

// Native variant uses milli::documents::Error
#[cfg(feature = "native")]
MalformedPayload(Error, PayloadType),
// WASM variant uses simplified error type
#[cfg(not(feature = "native"))]
MalformedPayload(DocumentPayloadError, PayloadType),
```

**`error.rs`** — 11 internal cfg gates:
```rust
#[cfg(feature = "native")]
use actix_web::{self as aweb, HttpResponseBuilder};
#[cfg(feature = "native")]
use milli::cellulite;
#[cfg(feature = "native")]
use milli::heed::{Error as HeedError, MdbError};

// ErrorCode impls for native types — gated
#[cfg(feature = "native")]
impl ErrorCode for file_store::Error { ... }
#[cfg(feature = "native")]
impl ErrorCode for HeedError { ... }
```

## 3. Remaining Fixes

### 3.1 Fix: `star_or.rs` — `crate::deserr` import

**Current (broken):**
```rust
use crate::deserr::query_params::FromQueryParameter;
// ^^^^^ `deserr` module is cfg(feature = "native")
```

**Fix:** Gate the import and all code using `FromQueryParameter` behind `#[cfg(feature = "native")]`.

```rust
#[cfg(feature = "native")]
use crate::deserr::query_params::FromQueryParameter;
```

The `FromQueryParameter` trait is used in `impl FromQueryParameter for OptionStarOr<T>` and `impl Deserr for OptionStarOr<T>` — both need cfg gating where they reference this trait.

**Decision D-1:** Gate `FromQueryParameter` integration behind `native`. This trait is only meaningful for actix-web query parameter parsing, which has no WASM use case. The core `StarOr<T>` and `OptionStarOr<T>` types (serde, Display, FromStr) remain WASM-safe.

### 3.2 Fix: `webhooks.rs` — `crate::settings::hide_secret`

**Current (broken):**
```rust
impl Webhook {
    pub fn redact_authorization_header(&mut self) {
        // ...
        crate::settings::hide_secret(value, "Bearer ".len());
        // ^^^^^^^^^^^^^^ `settings` module is cfg(feature = "native")
    }
}
```

**Fix:** Gate the `redact_authorization_header` method or provide an inline fallback.

**Decision D-2:** Gate the method behind `#[cfg(feature = "native")]`. On WASM, authorization header redaction is unnecessary (no server-side logging). Alternatively, inline the `hide_secret` logic:

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
            // Inline the secret hiding logic (avoid cross-module dependency)
            if value.len() > prefix_len {
                value.replace_range(prefix_len.., &"*".repeat(value.len() - prefix_len));
            }
        }
    }
}
```

**Preferred approach:** Inline the logic. It's 3 lines and removes the cross-module dependency entirely, making the method available on all targets.

### 3.3 Fix: `star_or.rs` — Trait Bound Issues

**Current (broken):**
```rust
// OptionStarOr impls that reference FromQueryParameter
impl<T, E> Deserr<E> for OptionStarOr<T>
where
    E: DeserializeError + MergeWithError<T::Err>,
    T: FromQueryParameter,  // <-- only available with native
```

**Fix:** Gate the `Deserr` impl for `OptionStarOr` that uses `FromQueryParameter` behind `native`, and provide a simpler WASM-safe impl if needed.

**Decision D-3:** The `Deserr` query parameter integration is native-only. Gate the full `impl Deserr<E> for OptionStarOr<T> where T: FromQueryParameter` behind `#[cfg(feature = "native")]`. The standard `FromStr`-based deserialization remains available on all targets.

## 4. WASM-Safe Type Inventory

After fixes, these types are available on all targets without `native`:

```rust
// error.rs
pub struct ResponseError { code, message, error_code, error_type, error_link }
pub enum Code { /* all error codes */ }
pub trait ErrorCode { fn error_code(&self) -> Code; }

// document_formats.rs
pub enum PayloadType { Ndjson, Json, Csv { delimiter } }
pub enum DocumentFormatError { Io(io::Error), MalformedPayload(...) }
// Note: read_csv, read_json, read_ndjson functions are native-only

// features.rs
pub struct RuntimeTogglableFeatures { metrics, logs_route, ... }
pub struct ChatCompletionSettings { source, ... }
// All chat-related settings types

// index_uid.rs
pub struct IndexUid(String)
pub struct IndexUidFormatError { invalid_uid }

// index_uid_pattern.rs
pub struct IndexUidPattern(String)

// network.rs
pub struct Network { local, remotes, leader }
pub struct Remote { url, search_api_key }

// star_or.rs (after fix)
pub enum StarOr<T> { Star, Other(T) }
pub enum OptionStarOr<T> { None, Star, Other(T) }
// Deserr query param integration is native-only

// webhooks.rs
pub struct Webhook { url, headers }
pub struct WebhooksView { webhooks }
pub struct WebhooksDumpView { webhooks }

// lib.rs
pub type Document = serde_json::Map<String, serde_json::Value>;
pub type InstanceUid = Uuid;
```

## 5. Design Decisions Log

| ID | Decision | Rationale |
|----|----------|-----------|
| D-1 | Gate `FromQueryParameter` integration behind `native` | Only meaningful for actix-web query parsing; no WASM use case |
| D-2 | Inline `hide_secret` in webhooks.rs | Removes cross-module dependency; 3 lines of string replacement logic |
| D-3 | Gate `Deserr<E> for OptionStarOr<T> where T: FromQueryParameter` behind `native` | This Deserr impl is HTTP-specific; standard FromStr works on all targets |
| D-4 | Don't add a `wasm` feature flag | The absence of `native` is sufficient; adding `wasm` creates confusing feature matrix |
| D-5 | Don't modify document_formats.rs or error.rs | Internal cfg gating is already comprehensive and well-tested |

## 6. Architecture Diagram

```
meilisearch-types
│
├── WASM-safe layer (always compiled) ──────────────────────────
│   ├── error.rs          ◄── ResponseError, Code, ErrorCode
│   │                         (native impls for milli/heed/file_store gated)
│   ├── document_formats.rs ◄── PayloadType, DocumentFormatError
│   │                         (native functions read_csv/json/ndjson gated)
│   ├── features.rs       ◄── RuntimeTogglableFeatures (pure serde)
│   ├── index_uid.rs      ◄── IndexUid (validation + serde)
│   ├── index_uid_pattern.rs ◄── IndexUidPattern (glob matching)
│   ├── network.rs        ◄── Network, Remote (pure serde)
│   ├── star_or.rs        ◄── StarOr, OptionStarOr
│   │                         (FromQueryParameter integration gated) ◄── FIX
│   └── webhooks.rs       ◄── Webhook, WebhooksView
│                              (hide_secret inlined) ◄── FIX
│
├── Native-only layer (#[cfg(feature = "native")]) ─────────────
│   ├── archive_ext.rs    ◄── tar archive unpacking with symlink safety
│   ├── batch_view.rs     ◄── milli progress types
│   ├── batches.rs        ◄── batch management types
│   ├── compression.rs    ◄── tar.gz filesystem operations
│   ├── deserr/           ◄── actix-web query/body deserialization
│   ├── facet_values_sort.rs ◄── milli::OrderBy wrapper
│   ├── keys.rs           ◄── API key types
│   ├── locales.rs        ◄── milli locale rules
│   ├── settings.rs       ◄── Full settings with milli types
│   ├── task_view.rs      ◄── Task display types
│   ├── tasks/            ◄── Task management types
│   └── versioning.rs     ◄── Database version file management
│
└── Re-exports
    ├── #[cfg(feature = "native")] pub use milli::{heed, Index};
    ├── #[cfg(feature = "native")] pub use {byte_unit, milli, serde_cs};
    └── #[cfg(not(feature = "native"))] pub use {byte_unit, serde_cs};
```

## 7. Consumer Impact

| Consumer | Impact | Action Required |
|----------|--------|-----------------|
| `dump` | None | Uses `native` features |
| `index-scheduler` | None | Uses `native` features |
| `meilisearch-auth` | None | Uses `native` features |
| `meilisearch` | None | Uses `native` features |
| `meilitool` | None | Uses `native` features |
| `wilysearch` | Positive | Can use `--no-default-features` for shared types |

## 8. Compilation Verification Matrix

| Command | Current | After Fix |
|---------|---------|-----------|
| `cargo check -p meilisearch-types` | PASS | PASS |
| `cargo check -p meilisearch-types --features native` | PASS | PASS |
| `cargo check -p meilisearch-types --no-default-features` | FAIL (6 errors) | PASS |
| `cargo check -p meilisearch-types --no-default-features --target wasm32-unknown-unknown` | FAIL | PASS |
| `cargo test -p meilisearch-types` | PASS | PASS |
