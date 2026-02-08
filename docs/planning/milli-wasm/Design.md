# milli WASM Compatibility — Technical Design

**Version:** 1.0.0
**Date:** 2026-02-07
**Status:** Draft
**Implements:** `Requirements.md` v1.0.0

---

## 1. Overview

This design introduces a **feature-flag layered abstraction** into the `milli` crate that replaces four hard native dependencies (LMDB, rayon, memmap2, tempfile) with trait-based abstractions. The native path uses the existing implementations; the WASM path uses in-memory/sequential alternatives. ML inference (candle) is excluded via conditional compilation.

### Design Principles

1. **Zero-cost on native:** Use `enum` dispatch (not `dyn Trait`) so the compiler monomorphizes away the abstraction layer.
2. **Minimal diff:** Preserve existing code structure; wrap rather than rewrite.
3. **Feature-flag isolation:** All WASM code lives behind `#[cfg(feature = "wasm")]` or `#[cfg(target_arch = "wasm32")]`.
4. **Codec reuse:** The `heed_codec` module is pure encode/decode — shared by all backends.

---

## 2. Architecture

```
┌─────────────────────────────────────────────────────────┐
│                     milli crate                          │
├─────────────────────────────────────────────────────────┤
│  Search Logic    │  Indexing Logic  │  Vector/Embedders  │
│  (pure Rust)     │  (needs parallel)│  (needs network)   │
├──────────────────┴─────────────────┴────────────────────┤
│                  Abstraction Layer                        │
│  ┌──────────┐ ┌───────────┐ ┌────────┐ ┌────────────┐  │
│  │ Storage  │ │ Executor  │ │ MMap   │ │ TempStore  │  │
│  │ Backend  │ │ (parallel)│ │ Compat │ │            │  │
│  └────┬─────┘ └─────┬─────┘ └───┬────┘ └─────┬──────┘  │
│       │             │            │             │          │
├───────┼─────────────┼────────────┼─────────────┼─────────┤
│ Native│        Native│       Native│        Native│       │
│ ┌─────┴───┐  ┌──────┴────┐ ┌────┴─────┐ ┌────┴──────┐  │
│ │  heed   │  │   rayon   │ │ memmap2  │ │ tempfile  │  │
│ │  LMDB   │  │ ThreadPool│ │  Mmap    │ │ NamedTemp │  │
│ └─────────┘  └───────────┘ └──────────┘ └───────────┘  │
│                                                          │
│ WASM  │        WASM  │       WASM  │        WASM  │      │
│ ┌─────┴───┐  ┌──────┴────┐ ┌────┴─────┐ ┌────┴──────┐  │
│ │ BTreeMap│  │Sequential │ │ Vec<u8>  │ │Cursor<Vec>│  │
│ │ in-mem  │  │  fallback │ │  buffer  │ │ in-memory │  │
│ └─────────┘  └───────────┘ └──────────┘ └───────────┘  │
└─────────────────────────────────────────────────────────┘
```

---

## 3. Feature Flag Design

### Cargo.toml Structure

```toml
[features]
default = ["native"]

# Native backend — current behavior
native = [
    "dep:heed", "dep:cellulite", "dep:rayon", "dep:memmap2", "dep:tempfile",
    "dep:candle-core", "dep:candle-nn", "dep:candle-transformers",
    "dep:hf-hub", "dep:safetensors", "dep:tokenizers", "dep:tiktoken-rs",
    "dep:arroy", "dep:hannoy",
    "dep:danger-ureq",
    "grenad/rayon", "grenad/tempfile",
    "indexmap/rayon",
]

# WASM backend — in-memory, sequential
wasm = [
    "dep:web-time",
    "dep:getrandom",
]

# WASM with threading (requires nightly + SharedArrayBuffer)
wasm-threads = ["wasm", "dep:wasm-bindgen-rayon"]

# Existing features unchanged
all-tokenizations = ["charabia/default"]
lmdb-posix-sem = ["heed?/posix-sem"]
cuda = ["candle-core?/cuda"]
enterprise = []
# ... tokenization features unchanged ...
```

### Dependency Sections

```toml
[dependencies]
# Always-available dependencies (pure Rust, WASM-compatible)
serde = { version = "1.0", features = ["derive"] }
roaring = { version = "0.10", features = ["serde"] }
fst = "0.4"
# ... (all pure-Rust deps listed in Requirements.md Section 6) ...

# Native-only dependencies (optional, enabled by `native` feature)
heed = { version = "0.22", optional = true, ... }
cellulite = { version = "0.3", optional = true }
rayon = { version = "1.11", optional = true }
memmap2 = { version = "0.9", optional = true }
tempfile = { version = "3.23", optional = true }
candle-core = { version = "0.9", optional = true }
candle-nn = { version = "0.9", optional = true }
candle-transformers = { version = "0.9", optional = true }
hf-hub = { ..., optional = true }
safetensors = { version = "0.6", optional = true }
tokenizers = { version = "0.22", optional = true, ... }
tiktoken-rs = { version = "0.9", optional = true }
arroy = { version = "0.6", optional = true }
hannoy = { version = "0.1", optional = true, ... }
danger-ureq = { ..., optional = true }

# WASM-only dependencies
web-time = { version = "1", optional = true }
getrandom = { version = "0.2", optional = true, features = ["js"] }
wasm-bindgen-rayon = { version = "1.3", optional = true }

# Grenad: features controlled by parent feature flags
grenad = { version = "0.5", default-features = false }
indexmap = { version = "2.12", features = ["serde"] }
flume = { version = "0.11", default-features = false }
```

### Decision: Feature vs cfg(target_arch)

**Context:** Should we use `#[cfg(feature = "wasm")]` or `#[cfg(target_arch = "wasm32")]`?

**Options:**
1. `cfg(feature = "wasm")` — explicit opt-in, works for cross-compilation testing on native
2. `cfg(target_arch = "wasm32")` — automatic, no feature flag needed
3. Both — feature flag for deps, target_arch for code paths

**Decision:** Option 3 (hybrid). Use `feature = "native"` / `feature = "wasm"` for dependency selection in Cargo.toml. Use `cfg(target_arch = "wasm32")` for inline code paths where the correct choice is always determined by target. This allows `cargo check --target wasm32-unknown-unknown --features wasm` to validate everything.

---

## 4. Component Designs

### 4.1 Storage Backend

**File:** `src/storage/mod.rs` (new module)

#### Trait Definition

```rust
use std::ops::RangeBounds;

/// Identifies a named database within the storage environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DatabaseId(pub(crate) u16);

/// Read-only transaction handle.
pub trait ReadTxn {
    fn get(&self, db: DatabaseId, key: &[u8]) -> Result<Option<&[u8]>>;
    fn iter<'txn>(&'txn self, db: DatabaseId)
        -> Result<Box<dyn Iterator<Item = (&'txn [u8], &'txn [u8])> + 'txn>>;
    fn range<'txn>(
        &'txn self,
        db: DatabaseId,
        range: impl RangeBounds<&'txn [u8]>,
    ) -> Result<Box<dyn Iterator<Item = (&'txn [u8], &'txn [u8])> + 'txn>>;
    fn prefix_iter<'txn>(
        &'txn self,
        db: DatabaseId,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = (&'txn [u8], &'txn [u8])> + 'txn>>;
    fn database_stat(&self, db: DatabaseId) -> Result<DatabaseStat>;
}

/// Read-write transaction handle.
pub trait WriteTxn: ReadTxn {
    fn put(&mut self, db: DatabaseId, key: &[u8], value: &[u8]) -> Result<()>;
    fn delete(&mut self, db: DatabaseId, key: &[u8]) -> Result<bool>;
    fn clear(&mut self, db: DatabaseId) -> Result<()>;
    fn commit(self) -> Result<()>;
    fn abort(self);
}

/// Storage environment (database container).
pub trait StorageEnv: Clone + Send + Sync {
    type RoTxn<'env>: ReadTxn where Self: 'env;
    type RwTxn<'env>: WriteTxn where Self: 'env;

    fn read_txn(&self) -> Result<Self::RoTxn<'_>>;
    fn write_txn(&self) -> Result<Self::RwTxn<'_>>;
    fn create_database(&self, name: Option<&str>) -> Result<DatabaseId>;
    fn database_stat(&self, txn: &Self::RoTxn<'_>, db: DatabaseId) -> Result<DatabaseStat>;
}

#[derive(Debug, Clone, Default)]
pub struct DatabaseStat {
    pub entries: usize,
    pub key_bytes: usize,
    pub value_bytes: usize,
}
```

#### Native Implementation (wraps heed)

**File:** `src/storage/lmdb.rs`

```rust
#[cfg(feature = "native")]
pub use heed::Env as LmdbEnv;
#[cfg(feature = "native")]
pub use heed::RoTxn as LmdbRoTxn;
#[cfg(feature = "native")]
pub use heed::RwTxn as LmdbRwTxn;
```

Rather than implementing the trait and adding indirection, the **native path uses type aliases**. The `Index` struct is generic over the storage env OR we use an enum-dispatch pattern:

```rust
/// Compile-time selected storage backend.
#[cfg(feature = "native")]
pub type Env = heed::Env<heed::WithoutTls>;
#[cfg(feature = "native")]
pub type RoTxn<'a> = heed::RoTxn<'a>;
#[cfg(feature = "native")]
pub type RwTxn<'a> = heed::RwTxn<'a>;

#[cfg(feature = "wasm")]
pub type Env = memory::MemoryEnv;
#[cfg(feature = "wasm")]
pub type RoTxn<'a> = memory::MemoryRoTxn<'a>;
#[cfg(feature = "wasm")]
pub type RwTxn<'a> = memory::MemoryRwTxn<'a>;
```

This avoids any trait overhead on native — it's just type aliases.

#### WASM Implementation (in-memory BTreeMap)

**File:** `src/storage/memory.rs`

```rust
#[cfg(feature = "wasm")]
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

/// In-memory storage environment.
#[derive(Clone)]
pub struct MemoryEnv {
    inner: Arc<RwLock<MemoryEnvInner>>,
}

struct MemoryEnvInner {
    databases: Vec<BTreeMap<Vec<u8>, Vec<u8>>>,
    db_names: BTreeMap<String, DatabaseId>,
    next_id: u16,
}

pub struct MemoryRoTxn<'env> {
    /// Snapshot: cloned BTreeMaps at transaction start for MVCC.
    snapshot: Vec<BTreeMap<Vec<u8>, Vec<u8>>>,
    _env: &'env MemoryEnv,
}

pub struct MemoryRwTxn<'env> {
    /// Working copy: cloned BTreeMaps, committed atomically.
    working: Vec<BTreeMap<Vec<u8>, Vec<u8>>>,
    env: &'env MemoryEnv,
}
```

**Serialization for snapshot transfer:**

```rust
impl MemoryEnv {
    /// Serialize entire environment to bytes for transfer to WASM.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let inner = self.inner.read().unwrap();
        bincode::serialize(&inner.databases)
    }

    /// Deserialize from bytes (e.g., loaded from fetch() in browser).
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let databases: Vec<BTreeMap<Vec<u8>, Vec<u8>>> = bincode::deserialize(data)?;
        // ... reconstruct env
    }
}
```

#### Decision: Generic Index vs Type Alias

**Context:** Should `Index` become `Index<E: StorageEnv>` or use conditional type aliases?

**Options:**
1. **Generic:** `pub struct Index<E: StorageEnv>` — maximum flexibility, viral generics
2. **Type alias:** Conditional `type Env = ...` — zero generics, compile-time selection
3. **Enum dispatch:** Runtime selection — unnecessary overhead

**Decision:** Option 2 (type alias). Generics would propagate through the entire codebase (~100+ function signatures). Type aliases keep the diff minimal and the compiler optimizes identically to today on native.

#### Handling the 25 Named Databases

The `Index` struct currently declares 25 `heed::Database<K, V>` fields with typed keys/values. In the type-alias approach:

```rust
// src/storage/database.rs

/// A typed database handle. On native, wraps heed::Database<K,V>.
/// On WASM, wraps a DatabaseId + phantom types for codec dispatch.
#[cfg(feature = "native")]
pub type Database<K, V> = heed::Database<K, V>;

#[cfg(feature = "wasm")]
pub struct Database<K, V> {
    id: DatabaseId,
    _key: PhantomData<K>,
    _value: PhantomData<V>,
}
```

The WASM `Database` uses the existing `heed_codec` types for serialization/deserialization — they're pure Rust encode/decode functions that work with `&[u8]`.

### 4.2 Parallelism Abstraction

**File:** `src/executor.rs` (new module)

#### ThreadPool Abstraction

```rust
/// Platform-aware task executor.
/// On native: wraps rayon::ThreadPool via ThreadPoolNoAbort.
/// On WASM: sequential single-thread execution.
#[cfg(feature = "native")]
pub use crate::thread_pool_no_abort::{
    ThreadPoolNoAbort as Executor,
    ThreadPoolNoAbortBuilder as ExecutorBuilder,
    PanicCatched,
};

#[cfg(feature = "wasm")]
pub struct Executor;

#[cfg(feature = "wasm")]
impl Executor {
    pub fn install<OP, R>(&self, op: OP) -> Result<R, PanicCatched>
    where
        OP: FnOnce() -> R + Send,
        R: Send,
    {
        // Execute synchronously — no thread pool in WASM.
        Ok(op())
    }

    pub fn broadcast<OP, R>(&self, op: OP) -> Result<Vec<R>, PanicCatched>
    where
        OP: Fn(BroadcastContext) -> R + Sync,
        R: Send,
    {
        // Single "thread" broadcast.
        Ok(vec![op(BroadcastContext { index: 0, num_threads: 1 })])
    }

    pub fn current_num_threads(&self) -> usize { 1 }
    pub fn active_operations(&self) -> usize { 0 }
}

#[cfg(feature = "wasm")]
pub struct ExecutorBuilder;

#[cfg(feature = "wasm")]
impl ExecutorBuilder {
    pub fn new() -> Self { Self }
    pub fn new_for_indexing() -> Self { Self }
    pub fn thread_name<F>(self, _: F) -> Self where F: FnMut(usize) -> String + 'static { self }
    pub fn num_threads(self, _: usize) -> Self { self }
    pub fn build(self) -> Result<Executor, ExecutorBuildError> { Ok(Executor) }
}

#[cfg(feature = "wasm")]
#[derive(Debug)]
pub struct PanicCatched;
impl std::fmt::Display for PanicCatched { ... }
impl std::error::Error for PanicCatched {}

#[cfg(feature = "wasm")]
pub struct BroadcastContext {
    pub index: usize,
    pub num_threads: usize,
}
```

#### Parallel Iterator Shim

For the 173 rayon call sites, provide a compatibility layer:

**File:** `src/executor/iter_compat.rs`

```rust
/// On WASM, provides sequential implementations that match rayon's API.
#[cfg(feature = "wasm")]
pub mod rayon_compat {
    pub trait IntoParallelIterator {
        type Item;
        type Iter: Iterator<Item = Self::Item>;
        fn into_par_iter(self) -> Self::Iter;
    }

    // Blanket impl: any IntoIterator becomes "parallel" (sequential).
    impl<I: IntoIterator> IntoParallelIterator for I {
        type Item = I::Item;
        type Iter = I::IntoIter;
        fn into_par_iter(self) -> Self::Iter {
            self.into_iter()
        }
    }

    pub trait ParallelIterator: Iterator {
        fn map<F, R>(self, f: F) -> std::iter::Map<Self, F>
        where Self: Sized, F: FnMut(Self::Item) -> R {
            Iterator::map(self, f)
        }
        // ... other methods mapping to sequential equivalents
    }

    impl<I: Iterator> ParallelIterator for I {}

    pub trait ParallelSliceMut<T> {
        fn par_sort_unstable_by_key<K, F>(&mut self, f: F)
        where K: Ord, F: Fn(&T) -> K;
    }

    impl<T> ParallelSliceMut<T> for [T] {
        fn par_sort_unstable_by_key<K, F>(&mut self, f: F)
        where K: Ord, F: Fn(&T) -> K {
            self.sort_unstable_by_key(f); // Sequential sort
        }
    }
}
```

Each file that uses rayon changes its imports:

```rust
// Before:
use rayon::iter::{IntoParallelIterator, ParallelIterator};

// After:
#[cfg(feature = "native")]
use rayon::iter::{IntoParallelIterator, ParallelIterator};
#[cfg(feature = "wasm")]
use crate::executor::rayon_compat::{IntoParallelIterator, ParallelIterator};
```

#### Decision: Shim rayon API vs. custom trait

**Context:** Should we shimmy rayon's exact API or define our own?

**Options:**
1. **Shim rayon API** — minimize code changes at call sites, just swap imports
2. **Custom trait** — cleaner abstraction, but touches every call site deeply

**Decision:** Option 1 (shim). The rayon API is used in 173 places. A shim that provides the same method signatures but runs sequentially minimizes the diff. Future `wasm-bindgen-rayon` support would just change the shim imports.

### 4.3 Memory-Mapped I/O Abstraction

**File:** `src/compat/mmap.rs` (new)

```rust
#[cfg(feature = "native")]
pub use memmap2::Mmap;
#[cfg(feature = "native")]
pub use memmap2::MmapMut;

#[cfg(feature = "wasm")]
pub struct Mmap {
    data: Vec<u8>,
}

#[cfg(feature = "wasm")]
impl Mmap {
    /// "Map" a file — in WASM, read entire contents into Vec.
    pub fn map(reader: &mut impl std::io::Read) -> std::io::Result<Self> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data)?;
        Ok(Self { data })
    }

    pub fn advise(&self, _advice: Advice) -> std::io::Result<()> {
        Ok(()) // No-op in WASM
    }
}

#[cfg(feature = "wasm")]
impl AsRef<[u8]> for Mmap {
    fn as_ref(&self) -> &[u8] { &self.data }
}

#[cfg(feature = "wasm")]
impl std::ops::Deref for Mmap {
    type Target = [u8];
    fn deref(&self) -> &[u8] { &self.data }
}

#[cfg(feature = "wasm")]
pub enum Advice { Sequential, WillNeed, Random }
```

The existing `ClonableMmap` wrapper (`Arc<Mmap>`) works unchanged since our WASM `Mmap` implements `AsRef<[u8]>`.

### 4.4 Temporary Storage Abstraction

**File:** `src/compat/tempfile.rs` (new)

```rust
#[cfg(feature = "native")]
pub fn tempfile() -> std::io::Result<std::fs::File> {
    tempfile::tempfile()
}

#[cfg(feature = "wasm")]
pub fn tempfile() -> std::io::Result<std::io::Cursor<Vec<u8>>> {
    Ok(std::io::Cursor::new(Vec::new()))
}

#[cfg(feature = "native")]
pub fn spooled_tempfile(max_size: usize) -> impl std::io::Write + std::io::Read + std::io::Seek {
    tempfile::SpooledTempFile::new(max_size)
}

#[cfg(feature = "wasm")]
pub fn spooled_tempfile(_max_size: usize) -> std::io::Cursor<Vec<u8>> {
    std::io::Cursor::new(Vec::new())
}
```

Call sites change from `tempfile::tempfile()` to `crate::compat::tempfile()`.

### 4.5 Time Abstraction

**File:** `src/compat/time.rs` (new)

```rust
#[cfg(not(target_arch = "wasm32"))]
pub use std::time::Instant;

#[cfg(target_arch = "wasm32")]
pub use web_time::Instant;
```

All 14 `std::time::Instant` usages change to `crate::compat::time::Instant`.

### 4.6 Embedder Conditional Compilation

**File:** `src/vector/embedder/mod.rs` (modified)

```rust
#[cfg(feature = "native")]
pub mod hf;

pub mod manual;
pub mod ollama;
pub mod openai;
pub mod rest;
pub mod composite;

#[derive(Debug)]
pub enum Embedder {
    #[cfg(feature = "native")]
    HuggingFace(hf::Embedder),
    OpenAi(openai::Embedder),
    UserProvided(manual::Embedder),
    Ollama(ollama::Embedder),
    Rest(rest::Embedder),
    Composite(composite::Embedder),
}
```

The `EmbedderOptions` enum mirrors this:

```rust
pub enum EmbedderOptions {
    #[cfg(feature = "native")]
    HuggingFace(hf::EmbedderOptions),
    OpenAi(openai::EmbedderOptions),
    UserProvided(manual::EmbedderOptions),
    Ollama(ollama::EmbedderOptions),
    Rest(rest::EmbedderOptions),
    Composite(composite::SubEmbedderOptions),
}
```

**Error types** in `src/vector/error.rs`:

```rust
#[cfg(feature = "native")]
TensorShape(candle_core::Error),
#[cfg(feature = "native")]
TensorValue(candle_core::Error),
// ... etc
```

### 4.7 Vector Store (arroy/hannoy)

**Decision:** Exclude vector store from WASM v1.

**Rationale:** `arroy` and `hannoy` are deeply coupled to heed (LMDB transactions). They are Meilisearch-maintained forks, but abstracting their storage is a separate large effort. For WASM v1:

```rust
// src/vector/mod.rs
#[cfg(feature = "native")]
pub mod db;  // arroy/hannoy based vector store

#[cfg(feature = "wasm")]
pub mod db {
    // Stub: vector search not available in WASM v1.
    // API-based embedders can still generate embeddings,
    // but ANN search is not supported client-side.
}
```

Future work can add a pure-Rust ANN index (e.g., `instant-distance` or custom HNSW).

---

## 5. Module Layout

```
src/
├── compat/                    # NEW: Platform compatibility layer
│   ├── mod.rs
│   ├── mmap.rs               # Mmap abstraction (REQ-MM-*)
│   ├── tempfile.rs            # TempFile abstraction (REQ-TF-*)
│   └── time.rs                # Instant abstraction (REQ-TI-*)
├── storage/                   # NEW: Storage backend abstraction
│   ├── mod.rs                 # Trait definitions + type aliases (REQ-ST-*)
│   ├── database.rs            # Typed Database handle
│   ├── lmdb.rs                # Native heed wrapper (REQ-ST-4)
│   └── memory.rs              # WASM in-memory backend (REQ-ST-5, REQ-ST-6)
├── executor/                  # NEW: Parallelism abstraction
│   ├── mod.rs                 # Executor type alias (REQ-PA-*)
│   └── rayon_compat.rs        # Sequential rayon API shim (REQ-PA-3, REQ-PA-5)
├── heed_codec/                # UNCHANGED — pure encode/decode (REQ-ST-8)
├── index.rs                   # MODIFIED — use storage type aliases
├── vector/
│   └── embedder/
│       ├── mod.rs             # MODIFIED — cfg-gate HuggingFace (REQ-ML-*)
│       ├── hf.rs              # UNCHANGED but cfg-gated
│       └── ...                # Other embedders unchanged
├── thread_pool_no_abort.rs    # UNCHANGED on native; aliased in executor/
├── update/                    # MODIFIED — swap imports throughout
└── search/                    # MODIFIED — swap imports for Instant, rayon
```

---

## 6. Migration Strategy for Existing Code

### Phase 1: Non-breaking additions

Add the `compat/`, `storage/`, and `executor/` modules. On native, they're pure type aliases or re-exports — zero behavioral change.

### Phase 2: Import swaps

Mechanically replace imports across the codebase:

| Before | After |
|--------|-------|
| `use std::time::Instant` | `use crate::compat::time::Instant` |
| `use memmap2::Mmap` | `use crate::compat::mmap::Mmap` |
| `use tempfile::tempfile` | `use crate::compat::tempfile::tempfile` |
| `use rayon::iter::*` | `#[cfg(native)] use rayon::*; #[cfg(wasm)] use crate::executor::rayon_compat::*` |
| `use heed::{RoTxn, RwTxn}` | `use crate::storage::{RoTxn, RwTxn}` |

### Phase 3: Index struct adaptation

Replace `heed::Database<K, V>` fields with `crate::storage::Database<K, V>`. On native this is a type alias to `heed::Database<K, V>` — zero change.

### Phase 4: WASM implementations

Implement `MemoryEnv`, `MemoryRoTxn`, `MemoryRwTxn`, sequential `Executor`, WASM `Mmap`, WASM `tempfile`.

### Phase 5: Conditional embedder compilation

Add `#[cfg(feature = "native")]` to HuggingFace embedder, candle error types, and vector store modules.

---

## 7. Error Handling

### Storage Errors

```rust
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("database not found: {0}")]
    DatabaseNotFound(String),
    #[error("key too large: {len} bytes (max: {max})")]
    KeyTooLarge { len: usize, max: usize },
    #[error("transaction aborted")]
    Aborted,
    #[error("environment closed")]
    EnvClosed,
    #[cfg(feature = "native")]
    #[error(transparent)]
    Heed(#[from] heed::Error),
}
```

The existing `InternalError` enum gains a `Storage(StorageError)` variant.

---

## 8. Testing Strategy

### Native Tests (unchanged)

All existing tests run with `--features native` (default). Zero regressions.

### WASM Compilation Check (CI)

```yaml
- name: Check WASM compilation
  run: cargo check --target wasm32-unknown-unknown --features wasm --no-default-features
```

### WASM Unit Tests

```bash
wasm-pack test --headless --chrome --features wasm -- --test wasm_smoke
```

Tests:
1. Create `MemoryEnv`, open databases, put/get/iterate
2. Run a basic search against in-memory index
3. Verify serialization round-trip of `MemoryEnv`

### Benchmark Regression

Run existing benchmarks with `--features native` before and after the abstraction. Target: <5% regression (REQ-NF-4).

---

## 9. Binary Size Considerations

Estimated WASM binary size contributors:

| Component | Estimated Size (uncompressed) |
|-----------|-------------------------------|
| SurrealQL-equivalent parser (charabia + filter-parser) | ~1.5 MB |
| Roaring bitmaps | ~200 KB |
| FST operations | ~150 KB |
| Serde + JSON | ~300 KB |
| Search logic | ~500 KB |
| In-memory storage | ~100 KB |
| Misc (geojson, rstar, etc.) | ~500 KB |
| **Total estimate** | **~3.2 MB** |
| **Gzipped** | **~1.0–1.5 MB** |

Well within the 5 MB gzipped target (REQ-NF-1).

---

## 10. Decisions Log

| # | Decision | Rationale |
|---|----------|-----------|
| D-1 | Type aliases over generic `Index<E>` | Avoids viral generics across 100+ signatures; compiler monomorphizes identically |
| D-2 | Rayon API shim over custom trait | 173 call sites — shim minimizes diff; future wasm-bindgen-rayon drops in |
| D-3 | Exclude vector store from WASM v1 | arroy/hannoy too coupled to heed; separate effort |
| D-4 | `MemoryEnv` uses BTreeMap (not HashMap) | Preserves key ordering required by range/prefix iteration |
| D-5 | MVCC via snapshot clone in MemoryRoTxn | Simple, correct; acceptable for WASM scale (<100k docs) |
| D-6 | `web-time` crate for Instant | De-facto standard; used by tokio, wasm-bindgen ecosystem |
| D-7 | `bincode` for snapshot serialization | Already a dependency; compact binary format |
| D-8 | Hybrid cfg strategy (feature + target_arch) | Feature for deps, target_arch for code; cleanest separation |
