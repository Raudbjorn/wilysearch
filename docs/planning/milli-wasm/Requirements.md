# milli WASM Compatibility — Requirements Specification

**Version:** 1.0.0
**Date:** 2026-02-07
**Status:** Draft
**Crate:** `milli` v1.35.0 (Meilisearch core indexing engine)

---

## 1. Problem Statement

The `milli` crate is Meilisearch's core indexing and search library. It currently compiles exclusively for native targets due to hard dependencies on LMDB (C FFI), rayon (OS threads), memmap2 (OS mmap), and candle (native ML inference). Making `milli` compile to `wasm32-unknown-unknown` would enable browser-based search, edge computing, and embedded use cases without a server.

## 2. Stakeholders

| Role | Interest |
|------|----------|
| **Library consumer (browser)** | Run search queries against pre-built indexes in-browser |
| **Library consumer (edge)** | Run lightweight indexing/search on edge runtimes (Cloudflare Workers, Deno Deploy) |
| **Meilisearch maintainers** | Minimize divergence; feature-flag approach, not a fork |
| **Upstream dependency authors** | heed, grenad, arroy, hannoy, cellulite — may need patches or alternatives |

## 3. Scope

### In Scope

- Feature-flagged WASM compilation of the `milli` crate
- Storage backend abstraction replacing direct heed/LMDB usage
- Parallelism abstraction replacing direct rayon usage
- Memory-mapped I/O abstraction replacing memmap2
- Temporary storage abstraction replacing tempfile
- Conditional compilation of ML embedder (candle/HF) vs API-only embedders
- `std::time::Instant` replacement for WASM (14 occurrences across 10 files)

### Out of Scope

- Full Meilisearch server compilation to WASM
- `index-scheduler`, `meilisearch-auth`, `dump` crate WASM porting
- Browser UI or JavaScript SDK bindings (separate project)
- Performance parity with native (WASM will be slower; that's acceptable)

## 4. User Stories

**US-1:** As a web application developer, I want to load a pre-built Meilisearch index in the browser and run search queries against it, so that I can provide instant search without network latency to a server.

**US-2:** As an edge computing developer, I want to compile milli to WASM and run it in a Cloudflare Worker or Deno Deploy runtime, so that search can run at the network edge.

**US-3:** As a Meilisearch contributor, I want WASM support to be feature-flagged so that the existing native code path is unaffected and CI continues to validate both targets.

**US-4:** As a library consumer, I want to index small document collections (< 100k docs) directly in WASM, so that I don't need a server for small-scale use cases.

**US-5:** As a library consumer using vector search, I want to use API-based embedders (OpenAI, Ollama, REST) from WASM, so that I can have semantic search without local ML inference.

## 5. Requirements (EARS Format)

### 5.1 Feature Flag System

**REQ-FF-1:** The `milli` crate SHALL define a `wasm` feature flag that, when enabled, replaces native-only dependencies with WASM-compatible alternatives.

**REQ-FF-2:** WHEN the `wasm` feature is NOT enabled THEN the crate SHALL compile identically to today's behavior with zero performance regression.

**REQ-FF-3:** WHEN the `wasm` feature is enabled THEN the crate SHALL NOT depend on: `heed`, `cellulite`, `rayon`, `memmap2`, `tempfile`, `candle-core`, `candle-nn`, `candle-transformers`, `hf-hub`, `arroy`, `hannoy`.

**REQ-FF-4:** WHEN the `wasm` feature is enabled THEN `cargo check --target wasm32-unknown-unknown` SHALL succeed with zero errors.

**REQ-FF-5:** The default feature set SHALL NOT include `wasm` (native remains default).

### 5.2 Storage Backend Abstraction

**REQ-ST-1:** The crate SHALL define a `StorageBackend` trait that abstracts all key-value storage operations currently performed by heed/LMDB.

**REQ-ST-2:** The `StorageBackend` trait SHALL support:
- Get by key: `fn get(&self, db: DatabaseId, key: &[u8]) -> Result<Option<&[u8]>>`
- Put key-value: `fn put(&mut self, db: DatabaseId, key: &[u8], value: &[u8]) -> Result<()>`
- Delete by key: `fn delete(&mut self, db: DatabaseId, key: &[u8]) -> Result<()>`
- Iterate: `fn iter(&self, db: DatabaseId) -> Result<Box<dyn Iterator<Item = (&[u8], &[u8])>>>`
- Range iteration: `fn range(&self, db: DatabaseId, range: impl RangeBounds<[u8]>) -> Result<Box<dyn Iterator<Item = (&[u8], &[u8])>>>`
- Prefix iteration: `fn prefix_iter(&self, db: DatabaseId, prefix: &[u8]) -> Result<Box<dyn Iterator<Item = (&[u8], &[u8])>>>`

**REQ-ST-3:** The `StorageBackend` trait SHALL support read transactions (snapshot isolation) and write transactions (atomic commit/rollback).

**REQ-ST-4:** WHEN the `wasm` feature is disabled THEN a `LmdbBackend` implementation SHALL wrap the existing heed/LMDB code with zero behavioral change.

**REQ-ST-5:** WHEN the `wasm` feature is enabled THEN a `MemoryBackend` implementation SHALL provide an in-memory BTreeMap-based storage backend.

**REQ-ST-6:** The `MemoryBackend` SHALL support serialization to/from bytes (via serde or bincode) so that index snapshots can be transferred to WASM runtimes.

**REQ-ST-7:** The `Index` struct (25 named databases, `src/index.rs:126-194`) SHALL be parameterized over or internally dispatch to the `StorageBackend`, preserving all 25 database definitions.

**REQ-ST-8:** The `heed_codec` module (13 source files) SHALL continue to function for both backends. Codecs are pure encode/decode logic and SHALL NOT be duplicated.

### 5.3 Parallelism Abstraction

**REQ-PA-1:** The crate SHALL define an execution abstraction that replaces direct rayon usage (173 occurrences across the codebase).

**REQ-PA-2:** WHEN the `wasm` feature is disabled THEN the abstraction SHALL delegate to rayon with zero overhead beyond a trait method call.

**REQ-PA-3:** WHEN the `wasm` feature is enabled THEN the abstraction SHALL provide a sequential single-threaded fallback.

**REQ-PA-4:** The `ThreadPoolNoAbort` struct (`src/thread_pool_no_abort.rs`) SHALL be abstracted behind a trait with `install()`, `broadcast()`, and `current_num_threads()` methods.

**REQ-PA-5:** The `ParallelIteratorExt` trait (`src/update/new/parallel_iterator_ext.rs`) SHALL have a sequential equivalent for WASM.

**REQ-PA-6:** IF `wasm-bindgen-rayon` (v1.3.0) support is desired in the future THEN the abstraction SHALL allow a third implementation backed by Web Workers + SharedArrayBuffer, gated behind a `wasm-threads` feature flag.

**REQ-PA-7:** `grenad` dependency (`v0.5.0`) is used with features `["rayon", "tempfile"]`. WHEN the `wasm` feature is enabled THEN grenad SHALL be compiled with neither feature, using its sequential/in-memory mode.

### 5.4 Memory-Mapped I/O Abstraction

**REQ-MM-1:** The crate SHALL define a `MappedSlice` trait or type alias abstracting over `memmap2::Mmap` (66 occurrences).

**REQ-MM-2:** WHEN the `wasm` feature is disabled THEN `MappedSlice` SHALL be backed by `memmap2::Mmap` (zero-copy, OS-managed).

**REQ-MM-3:** WHEN the `wasm` feature is enabled THEN `MappedSlice` SHALL be backed by `Vec<u8>` (heap-allocated buffer).

**REQ-MM-4:** The `ClonableMmap` wrapper (`src/update/index_documents/helpers/clonable_mmap.rs`) SHALL work with both backends via `Arc<dyn AsRef<[u8]>>` or equivalent.

**REQ-MM-5:** `memmap2::Advice::Sequential` hints SHALL be no-ops in WASM mode.

### 5.5 Temporary Storage Abstraction

**REQ-TF-1:** The crate SHALL define a `TempFile` abstraction replacing `tempfile::tempfile()` (51 occurrences).

**REQ-TF-2:** WHEN the `wasm` feature is disabled THEN `TempFile` SHALL delegate to `tempfile::tempfile()`.

**REQ-TF-3:** WHEN the `wasm` feature is enabled THEN `TempFile` SHALL be backed by `std::io::Cursor<Vec<u8>>` (in-memory buffer).

**REQ-TF-4:** `tempfile::spooled_tempfile(SIZE)` calls SHALL map to `Cursor<Vec<u8>>` in WASM mode (always "spooled" in memory since there's no disk to spill to).

### 5.6 ML/Embedder Conditional Compilation

**REQ-ML-1:** WHEN the `wasm` feature is enabled THEN the HuggingFace embedder (`src/vector/embedder/hf.rs`) SHALL be excluded via `#[cfg(not(target_arch = "wasm32"))]`.

**REQ-ML-2:** WHEN the `wasm` feature is enabled THEN the `Embedder` enum SHALL NOT include the `HuggingFace` variant.

**REQ-ML-3:** The following embedders SHALL remain available in WASM mode: `OpenAi`, `Ollama`, `Rest`, `UserProvided`, `Composite` (when composed of WASM-compatible sub-embedders).

**REQ-ML-4:** WHEN the `wasm` feature is enabled THEN `candle-core`, `candle-nn`, `candle-transformers`, `hf-hub`, `safetensors`, and `tokenizers` dependencies SHALL be excluded.

**REQ-ML-5:** The vector error types (`src/vector/error.rs`) that wrap `candle_core::Error` SHALL be conditionally compiled.

### 5.7 Time/Instant Abstraction

**REQ-TI-1:** `std::time::Instant` (14 occurrences across 10 files) SHALL be replaced with a platform-aware wrapper. `std::time::Instant` is not available on `wasm32-unknown-unknown`.

**REQ-TI-2:** WHEN the `wasm` feature is enabled THEN the wrapper SHALL use `web_time::Instant` (pure Rust, uses `performance.now()` in browsers).

**REQ-TI-3:** The `Deadline` struct (`src/lib.rs:139-178`) SHALL work identically on both targets.

### 5.8 Additional Platform Concerns

**REQ-PC-1:** `std::fs::File` usage (28 occurrences across 26 files) SHALL be gated behind `#[cfg(not(target_arch = "wasm32"))]` or abstracted.

**REQ-PC-2:** `std::thread::Builder` usage (`src/update/new/indexer/mod.rs:7`) SHALL be conditional — WASM cannot spawn OS threads.

**REQ-PC-3:** `crossbeam-channel` usage SHALL be replaced with `flume` (already a dependency, already `default-features = false`) in WASM mode, or verified that crossbeam compiles for WASM.

**REQ-PC-4:** `ureq` (synchronous HTTP client, used by `hf-hub`) SHALL be excluded in WASM mode.

### 5.9 Testing

**REQ-TS-1:** CI SHALL include a `cargo check --target wasm32-unknown-unknown --features wasm` job that runs on every PR.

**REQ-TS-2:** Unit tests for storage backend, parallelism abstraction, and mmap abstraction SHALL pass on both native and WASM targets.

**REQ-TS-3:** Integration tests requiring filesystem or threading SHALL be gated behind `#[cfg(not(target_arch = "wasm32"))]`.

**REQ-TS-4:** A `wasm-pack test --headless --chrome` integration test SHALL verify basic index creation and search in a browser environment.

### 5.10 Non-Functional Requirements

**REQ-NF-1:** The WASM binary size for a minimal search-only build SHALL be under 5 MB (gzipped).

**REQ-NF-2:** Search latency on a 10k-document index in WASM SHALL be under 50ms for simple text queries (measured in Chrome, M1-class hardware).

**REQ-NF-3:** The in-memory storage backend SHALL handle indexes up to 100k documents without exceeding 512 MB of WASM linear memory.

**REQ-NF-4:** The abstraction layer overhead on native targets SHALL be less than 5% compared to direct heed/rayon calls (measured via existing benchmarks).

## 6. Dependency Impact Matrix

### Replaced in WASM mode (11 dependencies)

| Dependency | Occurrences | Replacement |
|------------|-------------|-------------|
| `heed` | 25+ impl blocks | `StorageBackend` trait → `MemoryBackend` |
| `cellulite` | Core Index struct | Included in storage abstraction |
| `rayon` | 173 calls | Sequential fallback / `wasm-bindgen-rayon` |
| `memmap2` | 66 uses | `Vec<u8>` buffers |
| `tempfile` | 51 uses | `Cursor<Vec<u8>>` |
| `candle-core` | 27 uses | Excluded (`cfg`) |
| `candle-nn` | 1 use | Excluded (`cfg`) |
| `candle-transformers` | 5 uses | Excluded (`cfg`) |
| `hf-hub` | model loading | Excluded (`cfg`) |
| `arroy` | vector store | Excluded or in-memory impl |
| `hannoy` | HNSW index | Excluded or in-memory impl |

### Unchanged in WASM mode (pure Rust, already compatible)

`serde`, `serde_json`, `roaring`, `fst`, `charabia`, `filter-parser`, `flatten-serde-json`, `csv`, `bincode`, `bimap`, `bstr`, `bytemuck`, `byteorder`, `concat-arrays`, `convert_case`, `deserr`, `either`, `fxhash`, `geojson`, `geoutils`, `hashbrown`, `indexmap` (without rayon feature), `itertools`, `json-depth-checker`, `levenshtein_automata`, `liquid`, `lru`, `memchr`, `obkv`, `once_cell`, `ordered-float`, `rhai`, `rstar`, `slice-group-by`, `smallstr`, `smallvec`, `smartstring`, `steppe`, `thiserror`, `time`, `tracing`, `twox-hash`, `url`, `utoipa`, `uuid`, `zerometry`, `geo-types`, `bumpalo`, `bumparaw-collections`, `enum-iterator`, `rustc-hash`

### Needs WASM-specific features

| Dependency | Change needed |
|------------|---------------|
| `grenad` | Disable `rayon` and `tempfile` features |
| `indexmap` | Disable `rayon` feature |
| `flume` | Already `default-features = false` — verify WASM compat |
| `rand` | May need `getrandom` with `js` feature |
| `bbqueue` | Verify WASM compilation |
| `crossbeam-channel` | Verify or replace with flume |

## 7. Constraints

**C-1:** Must not break any existing native functionality. Zero regressions on current test suite.

**C-2:** Must not require nightly Rust for the sequential WASM fallback. The `wasm-bindgen-rayon` path (with threads) may require nightly.

**C-3:** The `heed_codec` module is pure encode/decode logic — it SHALL be shared between backends, not duplicated.

**C-4:** Upstream dependencies (`arroy`, `hannoy`, `cellulite`) are Meilisearch-maintained forks. They can be modified but changes should be minimal.

**C-5:** The `Index` struct has 25 named databases. The storage abstraction must support named/typed database handles, not just a flat KV store.

## 8. Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Storage abstraction adds >5% overhead on native | Medium | High | Benchmark early; use enum dispatch not dyn trait |
| grenad without rayon/tempfile doesn't compile or work | Low | High | Test early; grenad is pure Rust and configurable |
| `wasm-bindgen-rayon` requires nightly forever | Medium | Low | Sequential fallback is the primary path |
| WASM binary too large (>10 MB) | Medium | Medium | Feature-flag aggressive tree-shaking; strip unused embedders |
| Upstream crates (arroy, hannoy) too coupled to heed | High | High | May need to exclude vector store entirely in v1 WASM |

## 9. Acceptance Criteria Summary

1. `cargo check --target wasm32-unknown-unknown --features wasm` passes
2. All existing native tests pass without the `wasm` feature
3. Basic search test passes in `wasm-pack test --headless --chrome`
4. Native benchmark regression < 5%
5. WASM binary < 5 MB gzipped (search-only build)
