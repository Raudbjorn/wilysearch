# Meilisearch WebAssembly Compilation Evaluation

## Status: NOT FEASIBLE (Full Server) / PARTIALLY FEASIBLE (Individual Crates)

Meilisearch is an embedded database with indexing, storage, and ML capabilities designed for native platforms. Full server compilation to WASM is blocked by fundamental architectural dependencies. However, specific subsystems can be extracted and compiled with refactoring.

---

## 1. Workspace Overview

**26 workspace members.** No existing WASM support anywhere except a single shim in `reqwest-eventsource`:

```rust
// external-crates/reqwest-eventsource/src/event_source.rs
#[cfg(not(target_arch = "wasm32"))]
use futures_core::future::BoxFuture;
#[cfg(target_arch = "wasm32")]
use futures_core::future::LocalBoxFuture;
```

No CI builds target WASM. No `.cargo/config.toml` WASM settings.

### Pure-Rust crates that compile to WASM today (no changes needed)

```bash
cargo check --target wasm32-unknown-unknown \
  --package filter-parser \
  --package flatten-serde-json \
  --package json-depth-checker \
  --package permissive-json-pointer
```

These have zero native/OS dependencies.

---

## 2. Deep-Dive: `meilisearch-types`

**Verdict: MEDIUM BLOCKERS — partial compilation possible with feature-gating**

### Blocking Dependencies

| Dependency | Status | Issue |
|------------|--------|-------|
| `tempfile` | INCOMPATIBLE | OS-level temporary files |
| `memmap2` | INCOMPATIBLE | OS mmap syscall |
| `tokio` (default features) | INCOMPATIBLE | Native async I/O runtime |
| `file-store` (local) | INCOMPATIBLE | 100% filesystem I/O |
| `flate2` | COMPATIBLE | Pure Rust gzip |
| `tar` | COMPATIBLE | Pure Rust |
| `serde_json` | COMPATIBLE | Pure Rust |

### Source Code Issues

**`compression.rs`** — direct filesystem I/O:

```rust
use std::fs::{create_dir_all, File};
// File::create(), File::open() — no filesystem in WASM
```

**`archive_ext.rs`** — filesystem traversal:

```rust
use std::fs::DirEntry;
// fs::read_dir(), path.canonicalize() — not available in WASM
```

**`document_formats.rs`** — memory-mapped file parsing:

```rust
use memmap2::Mmap;
let input = unsafe { Mmap::map(input)? }; // mmap syscall — WASM has no OS
```

**`versioning.rs`** — filesystem version migration:

```rust
use std::fs;
use tempfile::NamedTempFile;
```

### Workaround

Feature-gate native I/O behind `#[cfg(not(target_arch = "wasm32"))]` and provide in-memory alternatives:

```toml
# meilisearch-types/Cargo.toml
[features]
default = ["native-io"]
native-io = ["tempfile", "memmap2", "file-store"]
```

```rust
// document_formats.rs
#[cfg(not(target_arch = "wasm32"))]
fn read_json_mmap(file: &File) -> Result<Vec<u8>> {
    let mmap = unsafe { Mmap::map(file)? };
    // ... existing impl
}

#[cfg(target_arch = "wasm32")]
fn read_json_buffered(data: &[u8]) -> Result<Vec<u8>> {
    // Read from in-memory buffer instead
}
```

---

## 3. Deep-Dive: `milli` (Core Indexing Engine)

**Verdict: HARD BLOCKERS — architectural incompatibility**

This is the heart of Meilisearch and has the most severe WASM blockers.

### Blocking Dependencies

| Dependency | Severity | Issue |
|------------|----------|-------|
| `heed` (LMDB wrapper) | HARD | C FFI to LMDB; requires mmap + file I/O |
| `cellulite` (nested LMDB txns) | HARD | Custom LMDB transaction layer |
| `rayon` | HARD | Work-stealing threadpool; WASM is single-threaded |
| `memmap2` | HARD | OS-level memory mapping |
| `candle-core` / `candle-transformers` | HARD | Native ML inference (CUDA/CPU) |
| `grenad` | MEDIUM | External sorting with `tempfile` + `rayon` |
| `hf-hub` | MEDIUM | Filesystem model cache + network I/O |
| `tokenizers` | MEDIUM | Pure Rust but uses `onig` regex |
| `arroy` / `hannoy` | HARD | Vector search built on LMDB |

### Source Code Issues

**`index.rs`** — entire Index struct is LMDB-backed:

```rust
use heed::{Database, RoTxn, RwTxn, WithoutTls};
use cellulite::Cellulite;
// The entire data model is LMDB transactions
```

No equivalent WASM-compatible embedded KV store provides ACID transactions with nested read-write transaction support.

**`thread_pool_no_abort.rs`** — parallel indexing:

```rust
use rayon::{ThreadPool, ThreadPoolBuilder};
// All indexing is parallelized via rayon work-stealing
```

WASM has no native threads. Sequential fallback would work but with massive performance impact (10-100x slower).

**`update/index_documents/helpers/clonable_mmap.rs`** — memory-mapped chunks:

```rust
use memmap2::Mmap;
pub struct ClonableMmap { inner: Arc<Mmap> }
// grenad spills to temp files and mmaps them during indexing
```

**`vector/embedder/hf.rs`** — HuggingFace embedder:

```rust
use candle_core::Tensor;
use candle_transformers::models::bert::BertModel;
use hf_hub::api::sync::Api;
// Downloads and runs BERT/ModernBERT models locally
```

No WASM-compatible BERT inference engine exists. API-based embedders (OpenAI, Ollama, REST) would work.

### Workaround: Storage Backend Abstraction

```rust
pub trait StorageBackend: Send + Sync {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;
    fn put(&mut self, key: &[u8], val: &[u8]) -> Result<()>;
    fn delete(&mut self, key: &[u8]) -> Result<()>;
    fn iter(&self) -> Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)> + '_>;
}

#[cfg(target_arch = "wasm32")]
pub struct MemoryStorage {
    data: BTreeMap<Vec<u8>, Vec<u8>>,
}

#[cfg(not(target_arch = "wasm32"))]
pub struct LmdbStorage {
    env: heed::Env,
}
```

### Workaround: Parallelism Abstraction

```rust
pub trait Parallelizer {
    fn par_map<I, F, R>(&self, iter: I, f: F) -> Vec<R>
    where I: IntoIterator, F: Fn(I::Item) -> R + Send, R: Send;
}

#[cfg(target_arch = "wasm32")]
pub struct SequentialProcessor;
impl Parallelizer for SequentialProcessor {
    fn par_map<I, F, R>(&self, iter: I, f: F) -> Vec<R> {
        iter.into_iter().map(f).collect() // Sequential fallback
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub struct RayonProcessor(rayon::ThreadPool);
impl Parallelizer for RayonProcessor {
    fn par_map<I, F, R>(&self, iter: I, f: F) -> Vec<R> {
        self.0.install(|| iter.into_par_iter().map(f).collect())
    }
}
```

### Workaround: Feature-Flag Architecture

```toml
# milli/Cargo.toml
[features]
default = ["lmdb-backend", "native-ml"]
lmdb-backend = ["heed", "cellulite", "rayon", "memmap2"]
wasm-backend = []  # Pure Rust in-memory storage
native-ml = ["candle-core", "candle-transformers", "hf-hub"]
wasm-api-embeddings = []  # OpenAI/Ollama only, no local inference
```

---

## 4. Deep-Dive: `file-store`

**Verdict: HARD BLOCKER — 100% filesystem dependent**

Every function in this crate performs direct filesystem I/O:

```rust
use std::fs::File as StdFile;
use tempfile::NamedTempFile;

impl FileStore {
    pub fn new(path: impl AsRef<Path>) -> Result<FileStore> {
        std::fs::create_dir_all(&path)?;     // no fs in WASM
    }

    pub fn new_update(&self) -> Result<(Uuid, File)> {
        NamedTempFile::new_in(&self.path)?;   // no tempfile in WASM
    }

    pub fn get_update(&self, uuid: Uuid) -> Result<StdFile> {
        StdFile::open(path)?                  // no fs::File in WASM
    }

    pub fn snapshot(&self, uuid: Uuid, dst: impl AsRef<Path>) -> Result<()> {
        std::fs::copy(src, dst)?              // no fs copy in WASM
    }

    pub fn all_uuids(&self) -> Result<impl Iterator<Item = Result<Uuid>>> {
        self.path.read_dir()?                 // no read_dir in WASM
    }
}
```

### Workaround: Abstract Storage Trait

```rust
pub trait UpdateStorage {
    fn new_update(&self) -> Result<(Uuid, Box<dyn Write>)>;
    fn get_update(&self, uuid: Uuid) -> Result<Box<dyn Read>>;
    fn delete(&self, uuid: Uuid) -> Result<()>;
    fn all_uuids(&self) -> Result<Vec<Uuid>>;
}

#[cfg(target_arch = "wasm32")]
pub struct MemoryUpdateStorage {
    files: Arc<RwLock<HashMap<Uuid, Vec<u8>>>>,
}

#[cfg(not(target_arch = "wasm32"))]
pub struct FileUpdateStorage { /* existing FileStore */ }
```

---

## 5. Deep-Dive: `http-client`

**Verdict: MEDIUM — mostly compatible, needs conditional compilation**

### Dependencies

| Dependency | WASM Status | Notes |
|------------|-------------|-------|
| `reqwest` | COMPATIBLE | Official WASM support via `web-sys` Fetch API |
| `ureq` | INCOMPATIBLE | Blocking sync HTTP using `std::net::TcpStream` |
| `hyper-util` | PARTIAL | Requires async runtime |
| `cidr` | COMPATIBLE | Pure Rust IP validation |

### Source Code

**`reqwest/mod.rs`** — already WASM-compatible. Uses `rustls-tls-native-roots` which needs swapping to `rustls-tls` for WASM (no native root certificates in browser).

**`ureq/mod.rs`** — fully incompatible. Synchronous HTTP over OS sockets.

**`policy.rs`** — pure IP parsing logic, already WASM-compatible.

### Workaround

```rust
// http-client/src/lib.rs
#[cfg(not(target_arch = "wasm32"))]
pub mod ureq;

// Always available — reqwest supports WASM natively
pub mod reqwest;

#[cfg(not(target_arch = "wasm32"))]
pub mod policy; // If policy.rs uses OS-specific DNS resolution
```

```toml
# http-client/Cargo.toml
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
ureq = "3.1"

[target.'cfg(target_arch = "wasm32")'.dependencies]
reqwest = { version = "0.12", features = ["rustls-tls"] }

[target.'cfg(not(target_arch = "wasm32"))'.dependencies.reqwest]
version = "0.12"
features = ["rustls-tls-native-roots"]
```

---

## 6. Blocking Dependencies Matrix

### HARD (architectural redesign required)

| Dependency | Crate | WASM Alternative |
|------------|-------|------------------|
| LMDB/Cellulite | milli | In-memory BTreeMap or IndexedDB via js-sys |
| Rayon | milli | Sequential processing (10-100x slower) |
| memmap2 | milli, meilisearch-types | Buffer in linear memory |
| tempfile | file-store, meilisearch-types | `Vec<u8>` or web storage |
| Candle | milli (HF embedder) | Disable; use API-based embedders only |
| ureq | http-client | Remove in WASM build; use reqwest only |

### MEDIUM (feature flags or dependency swaps)

| Dependency | Crate | Workaround |
|------------|-------|------------|
| hf-hub | milli | Gate behind cfg; async-only in WASM |
| tokenizers | milli | Use pure Rust regex instead of onig |
| std::fs | widespread | Abstract behind trait + feature flag |
| tokio (full) | meilisearch-types | Use `macros` feature only |

### EASY (already compatible)

| Dependency | Notes |
|------------|-------|
| flate2, tar | Pure Rust |
| serde_json | Pure Rust |
| liquid | Pure Rust template engine |
| rhai | Scripting engine |
| cidr | IP parsing |

---

## 7. Realistic Compilation Scenarios

### Full server → WASM

```
cargo check --target wasm32-unknown-unknown
```

**Result: FAILS.** Blocked by heed/LMDB, rayon, memmap2, tempfile, candle, ureq.

### `meilisearch-types` only (without file-store)

```
cargo check --target wasm32-unknown-unknown --package meilisearch-types --no-default-features
```

**Result: PARTIAL FAILURE.** Compiles document format parsing (JSON/CSV) if memmap2 is removed. Fails on `compression.rs` (tar.gz on files) and `versioning.rs` (filesystem migrations).

### Pure-Rust utility crates

```
cargo check --target wasm32-unknown-unknown \
  --package filter-parser \
  --package flatten-serde-json \
  --package json-depth-checker \
  --package permissive-json-pointer
```

**Result: SUCCESS.** These have no native/OS dependencies.

---

## 8. Practical Approaches

### Option A: In-Memory Search Engine (2-4 weeks)

Extract `milli/src/search/new` module (search algorithm logic). Implement `InMemoryIndex` backed by `BTreeMap`. Ship pre-built index snapshots from server. Compile search logic to WASM for client-side queries. Avoids all HARD blockers.

### Option B: Hybrid Index + Search (4-6 weeks)

Split milli into `milli-search` + `milli-index`. Compile both to WASM with:
- Sequential processing (skip rayon)
- In-memory storage (skip LMDB)
- API-only embeddings (skip candle/HF)
- Buffered I/O (skip mmap)

### Option C: Document Parser Library (1-2 weeks)

Extract document format parsing (JSON, CSV, NDJSON) into standalone WASM library. Handle format conversion in browser before sending to server.

### Option D: Full Server WASM (6-12+ months)

Replace LMDB entirely with a pure-Rust KV store. Replace rayon with sequential processing. Strip all native ML. Essentially a new codebase. **Not recommended.**

---

## 9. Effort Summary

| Approach | Effort | Feasibility |
|----------|--------|-------------|
| Full WASM server | 6-12+ months | Not practical |
| In-memory search lib | 2-4 weeks | Recommended |
| Hybrid index + search | 4-6 weeks | Moderate |
| Document parser lib | 1-2 weeks | Easiest |
| Utility crates only | 0 (works today) | Already done |

---

## 10. Conclusion

Meilisearch is a native-first system. The core indexing engine (`milli`) is deeply coupled to LMDB (C FFI), rayon (OS threads), and memmap2 (OS memory mapping) — none of which exist in WASM.

**What works today:** Pure-Rust utility crates (filter-parser, flatten-serde-json, json-depth-checker).

**What's achievable with moderate effort:** Client-side search over pre-built index snapshots, document format parsing, HTTP client (reqwest path only).

**What requires architectural redesign:** Anything involving indexing, storage, or local ML inference.

The most practical path is extracting the search algorithm into a standalone WASM library that operates over serialized index snapshots, avoiding all storage and threading blockers entirely.
