# milli WASM Compatibility — Implementation Tasks

**Version:** 1.0.0
**Date:** 2026-02-07
**Status:** Draft
**Implements:** `Design.md` v1.0.0
**Traceability:** Each task references Requirements (REQ-*) and Design decisions (D-*)

---

## Task Sequencing Strategy: Foundation-First

Tasks are ordered so that each builds on the previous, with early tasks providing the abstraction layer and later tasks consuming it. The sequence is:

1. **Infrastructure** — feature flags, compat module, CI
2. **Storage abstraction** — the largest and most critical piece
3. **Parallelism abstraction** — rayon shim
4. **I/O abstractions** — mmap, tempfile, time
5. **Import migration** — mechanical replacement across codebase
6. **Conditional compilation** — embedders, vector store
7. **WASM implementations** — MemoryEnv, sequential executor
8. **Testing & validation** — CI, wasm-pack tests, benchmarks

---

## Epic 1: Infrastructure & Feature Flags

### Task 1.1: Add feature flag structure to Cargo.toml
- Add `native` and `wasm` feature flags per Design Section 3
- Make all LMDB/rayon/memmap2/tempfile/candle deps optional behind `native`
- Add `web-time` and `getrandom` as optional behind `wasm`
- Set `default = ["native"]`
- Configure `grenad` and `indexmap` features conditionally
- Verify `cargo check --features native` still passes (no regression)
- **Files:** `crates/milli/Cargo.toml`
- _Requirements: REQ-FF-1, REQ-FF-2, REQ-FF-3, REQ-FF-5_

### Task 1.2: Create `src/compat/` module skeleton
- Create `src/compat/mod.rs` with `pub mod mmap; pub mod tempfile; pub mod time;`
- Each submodule initially just re-exports the native type (passthrough)
- Verify compilation unchanged
- **Files:** `src/compat/mod.rs`, `src/compat/mmap.rs`, `src/compat/tempfile.rs`, `src/compat/time.rs`
- _Requirements: REQ-MM-1, REQ-TF-1, REQ-TI-1_

### Task 1.3: Create `src/storage/` module skeleton
- Create `src/storage/mod.rs` with trait definitions per Design Section 4.1
- Create `src/storage/database.rs` with typed Database handle
- Native path: type aliases to heed types
- Verify compilation unchanged
- **Files:** `src/storage/mod.rs`, `src/storage/database.rs`
- _Requirements: REQ-ST-1, REQ-ST-2, REQ-ST-3_

### Task 1.4: Create `src/executor/` module skeleton
- Create `src/executor/mod.rs` with Executor type alias
- Native path: re-export `ThreadPoolNoAbort` as `Executor`
- Verify compilation unchanged
- **Files:** `src/executor/mod.rs`
- _Requirements: REQ-PA-1, REQ-PA-2, REQ-PA-4_

### Task 1.5: Add CI WASM check job
- Add `cargo check --target wasm32-unknown-unknown --features wasm --no-default-features` to CI
- Initially this will fail — subsequent tasks make it pass
- Add `rustup target add wasm32-unknown-unknown` step
- **Files:** `.github/workflows/ci.yml`
- _Requirements: REQ-TS-1_

---

## Epic 2: Storage Backend Abstraction

### Task 2.1: Define storage type aliases for native
- In `src/storage/mod.rs`, define:
  - `type Env = heed::Env<WithoutTls>`
  - `type RoTxn<'a> = heed::RoTxn<'a>`
  - `type RwTxn<'a> = heed::RwTxn<'a>`
- Gate behind `#[cfg(feature = "native")]`
- **Files:** `src/storage/mod.rs`
- _Requirements: REQ-ST-4, Design D-1_

### Task 2.2: Define `Database<K, V>` wrapper type
- Native: `type Database<K, V> = heed::Database<K, V>`
- WASM: `struct Database<K, V> { id: DatabaseId, _key: PhantomData<K>, _value: PhantomData<V> }`
- Ensure codec compatibility with `heed_codec` types
- **Files:** `src/storage/database.rs`
- _Requirements: REQ-ST-7, REQ-ST-8_

### Task 2.3: Migrate `Index` struct to use storage types
- Replace `use heed::{Database, Env, RoTxn, RwTxn, ...}` with `use crate::storage::*`
- Replace `heed::Env<WithoutTls>` field with `crate::storage::Env`
- Replace all 25 `Database<K, V>` field types
- Replace `Index::new_with_creation_dates` to use storage types
- Verify all existing tests pass
- **Files:** `src/index.rs` (primary), all files importing from index
- _Requirements: REQ-ST-7, REQ-FF-2_

### Task 2.4: Migrate `Index` method implementations
- Walk through all `impl Index` methods
- Replace `RoTxn`/`RwTxn` parameter types with storage aliases
- This is the largest mechanical change — ~50+ methods
- Run full test suite after
- **Files:** `src/index.rs` (bulk of changes)
- _Requirements: REQ-ST-4_

### Task 2.5: Migrate search subsystem to storage types
- `src/search/new/db_cache.rs` — uses `RoTxn<'_>`
- `src/search/new/` — all ranking rules and search context
- `src/search/facet/` — facet search operations
- Replace heed imports with storage imports
- **Files:** `src/search/**/*.rs`
- _Requirements: REQ-ST-4_

### Task 2.6: Migrate update subsystem to storage types
- `src/update/new/channel.rs` — `Database` enum wrapping heed types
- `src/update/new/merger.rs` — `FacetDatabases`
- `src/update/new/indexer/mod.rs` — main `index()` function
- `src/update/settings.rs` — settings operations
- `src/update/upgrade/` — migration code
- Replace all heed imports with storage imports
- **Files:** `src/update/**/*.rs`
- _Requirements: REQ-ST-4_

### Task 2.7: Implement `MemoryEnv` for WASM
- Implement `MemoryEnv`, `MemoryRoTxn`, `MemoryRwTxn` per Design Section 4.1
- BTreeMap-based storage with snapshot MVCC
- Implement `to_bytes()` / `from_bytes()` serialization
- Support all 25 named databases
- Gate behind `#[cfg(feature = "wasm")]`
- Write unit tests for the memory backend
- **Files:** `src/storage/memory.rs`
- _Requirements: REQ-ST-5, REQ-ST-6, Design D-4, D-5_

---

## Epic 3: Parallelism Abstraction

### Task 3.1: Create rayon API shim for WASM
- Implement sequential equivalents of rayon traits per Design Section 4.2
- `IntoParallelIterator` → delegates to `IntoIterator`
- `ParallelIterator` → delegates to `Iterator`
- `IndexedParallelIterator` → delegates to `Iterator + ExactSizeIterator`
- `par_sort_unstable_by_key` → `sort_unstable_by_key`
- `par_chunks` → `chunks`
- Gate behind `#[cfg(feature = "wasm")]`
- **Files:** `src/executor/rayon_compat.rs`
- _Requirements: REQ-PA-3, REQ-PA-5, Design D-2_

### Task 3.2: Create WASM Executor (sequential ThreadPool)
- Implement `Executor` struct with `install()`, `broadcast()`, `current_num_threads()` per Design Section 4.2
- `install()` runs the closure synchronously
- `broadcast()` runs once with index=0
- `current_num_threads()` returns 1
- Gate behind `#[cfg(feature = "wasm")]`
- **Files:** `src/executor/mod.rs`
- _Requirements: REQ-PA-3, REQ-PA-4_

### Task 3.3: Migrate rayon imports across codebase
- Replace `use rayon::iter::*` with conditional imports in all 27 files
- Replace `rayon::current_num_threads()` with `Executor::current_num_threads()`
- Update `ThreadPoolNoAbort` references to `Executor`
- Files with most rayon usage (prioritize):
  - `src/update/new/indexer/mod.rs` (core indexing)
  - `src/update/new/indexer/document_operation.rs`
  - `src/update/new/merger.rs`
  - `src/update/new/words_prefix_docids.rs`
  - `src/update/upgrade/v1_32.rs`
  - `src/vector/embedder/rest.rs`, `openai.rs`, `ollama.rs`
- Run test suite after each batch
- **Files:** ~27 files with rayon imports
- _Requirements: REQ-PA-1, REQ-PA-2_

### Task 3.4: Handle `grenad` feature configuration
- When `wasm` feature: `grenad = { default-features = false }` (no rayon, no tempfile)
- Verify grenad compiles and functions in sequential mode
- Grenad's `Writer` and `Sorter` should fall back to in-memory sorting
- **Files:** `Cargo.toml`, potentially grenad integration points
- _Requirements: REQ-PA-7_

---

## Epic 4: I/O Abstractions

### Task 4.1: Implement WASM `Mmap` compatibility type
- Per Design Section 4.3: `Mmap` backed by `Vec<u8>`
- Implement `AsRef<[u8]>`, `Deref<Target=[u8]>`, `map()` from reader
- `Advice` enum with no-op `advise()` method
- MmapMut backed by `Vec<u8>` with `as_mut()` / `flush()` as no-ops
- Gate behind `#[cfg(feature = "wasm")]`
- **Files:** `src/compat/mmap.rs`
- _Requirements: REQ-MM-1, REQ-MM-2, REQ-MM-3, REQ-MM-5_

### Task 4.2: Migrate memmap2 imports (66 occurrences)
- Replace `use memmap2::Mmap` with `use crate::compat::mmap::Mmap`
- Replace `memmap2::Advice` with `crate::compat::mmap::Advice`
- Verify `ClonableMmap` still works (uses `Arc<Mmap>` + `AsRef<[u8]>`)
- **Files:** 10 files using memmap2
- _Requirements: REQ-MM-4_

### Task 4.3: Implement WASM tempfile compatibility
- Per Design Section 4.4: `tempfile()` returns `Cursor<Vec<u8>>`
- `spooled_tempfile()` returns `Cursor<Vec<u8>>`
- Gate behind `#[cfg(feature = "wasm")]`
- **Files:** `src/compat/tempfile.rs`
- _Requirements: REQ-TF-2, REQ-TF-3, REQ-TF-4_

### Task 4.4: Migrate tempfile imports (51 occurrences)
- Replace `use tempfile::tempfile` with `use crate::compat::tempfile::tempfile`
- Replace `tempfile::spooled_tempfile` with `use crate::compat::tempfile::spooled_tempfile`
- Handle `TempDir` usages (test-only — gate behind `#[cfg(test)]` + `#[cfg(not(wasm))]`)
- **Files:** ~21 files using tempfile
- _Requirements: REQ-TF-1_

### Task 4.5: Implement time::Instant compatibility
- Per Design Section 4.5: `web_time::Instant` on WASM
- **Files:** `src/compat/time.rs`
- _Requirements: REQ-TI-2_

### Task 4.6: Migrate `std::time::Instant` imports (14 occurrences)
- Replace across 10 files
- Verify `Deadline` struct compiles on both targets
- **Files:** `src/lib.rs`, `src/search/hybrid.rs`, `src/update/facet/mod.rs`, `src/search/new/logger/visual.rs`, `src/search/new/vector_sort.rs`, `src/vector/embedder/mod.rs`, `src/vector/embedder/openai.rs`, `src/vector/embedder/ollama.rs`, `src/vector/embedder/composite.rs`, `src/vector/embedder/rest.rs`
- _Requirements: REQ-TI-3_

---

## Epic 5: Conditional Compilation — Embedders & Vector Store

### Task 5.1: Gate HuggingFace embedder behind `native` feature
- Add `#[cfg(feature = "native")]` to `pub mod hf` in embedder/mod.rs
- Gate `Embedder::HuggingFace` variant
- Gate `EmbedderOptions::HuggingFace` variant
- Handle serde deserialization for unknown variants (WASM receiving HF config should error gracefully)
- **Files:** `src/vector/embedder/mod.rs`, `src/vector/embedder/hf.rs`
- _Requirements: REQ-ML-1, REQ-ML-2, REQ-ML-3_

### Task 5.2: Gate candle error types
- Add `#[cfg(feature = "native")]` to all `candle_core::Error` wrapping variants in `src/vector/error.rs`
- 8 variants to gate: `TensorShape`, `TensorValue`, `ModelForward`, `PytorchWeight`, `SafetensorWeight`, `LoadModel`, etc.
- **Files:** `src/vector/error.rs`
- _Requirements: REQ-ML-5_

### Task 5.3: Gate vector store (arroy/hannoy)
- Add `#[cfg(feature = "native")]` to vector store database module
- Add `#[cfg(feature = "native")]` to `hannoy::Database` field in `Index` struct
- Provide stub or empty module for WASM
- Gate `Cellulite` field in `Index` struct
- **Files:** `src/vector/db.rs`, `src/index.rs`
- _Requirements: Design D-3_

### Task 5.4: Gate `std::fs::File` usage (28 occurrences)
- Most are in update/indexing paths — already covered by tempfile/mmap migration
- Remaining direct `std::fs::File` usage in `src/index.rs:5` (compaction), `src/documents/enriched.rs`, `src/vector/embedder/hf.rs` (already gated)
- Gate with `#[cfg(not(target_arch = "wasm32"))]` where appropriate
- **Files:** ~26 files
- _Requirements: REQ-PC-1_

### Task 5.5: Gate `std::thread::Builder` and `crossbeam-channel`
- `std::thread::Builder` in `src/update/new/indexer/mod.rs:7` — gate behind native
- Verify `crossbeam-channel` compiles for wasm32 OR replace with `flume`
- Verify `bbqueue` compiles for wasm32
- **Files:** `src/update/new/indexer/mod.rs`, Cargo.toml
- _Requirements: REQ-PC-2, REQ-PC-3_

### Task 5.6: Gate `ureq` dependency
- Used by `hf-hub` (already gated with HF embedder) and directly as `danger-ureq`
- Ensure it's excluded when `native` feature is off
- **Files:** `Cargo.toml`
- _Requirements: REQ-PC-4_

---

## Epic 6: Testing & Validation

### Task 6.1: Verify `cargo check --target wasm32-unknown-unknown --features wasm`
- Run the check and fix any remaining compilation errors
- This is the critical gate — iterate until it passes
- **Files:** Various (fixing any missed cfg gates)
- _Requirements: REQ-FF-4, REQ-TS-1_

### Task 6.2: Write MemoryEnv unit tests
- Test all CRUD operations across multiple databases
- Test transaction isolation (read snapshot doesn't see uncommitted writes)
- Test transaction commit and abort
- Test range and prefix iteration
- Test serialization round-trip
- **Files:** `src/storage/memory.rs` (test module)
- _Requirements: REQ-TS-2_

### Task 6.3: Write WASM smoke test with wasm-pack
- Create `tests/wasm_smoke.rs`
- Test: create MemoryEnv → add documents → run search → verify results
- Run with `wasm-pack test --headless --chrome`
- **Files:** `tests/wasm_smoke.rs`
- _Requirements: REQ-TS-4_

### Task 6.4: Gate existing integration tests for native-only
- Add `#[cfg(feature = "native")]` to tests that use filesystem or threading
- Ensure `cargo test --features native` runs all existing tests
- **Files:** `src/test_index.rs`, `src/search/new/tests/integration.rs`, various test modules
- _Requirements: REQ-TS-3_

### Task 6.5: Run native benchmarks — regression check
- Run existing benchmarks with `--features native`
- Compare against baseline (pre-abstraction)
- Target: <5% regression
- If regression exceeds 5%, profile and optimize the abstraction layer
- **Files:** `benches/`
- _Requirements: REQ-NF-4_

### Task 6.6: Measure WASM binary size
- Build: `cargo build --target wasm32-unknown-unknown --features wasm --release --no-default-features`
- Run `wasm-opt -Oz` on the output
- Measure gzipped size
- Target: <5 MB gzipped
- If too large, identify and exclude unnecessary modules
- _Requirements: REQ-NF-1_

### Task 6.7: Update CI workflow
- Finalize the WASM check job from Task 1.5 (should now pass)
- Add `wasm-pack test` job
- Ensure native test suite still runs unchanged
- **Files:** `.github/workflows/ci.yml`
- _Requirements: REQ-TS-1_

---

## Dependency Graph

```
1.1 ─── Feature flags
 │
 ├── 1.2 ─── compat/ skeleton
 │    │
 │    ├── 4.1 ─── WASM Mmap impl
 │    ├── 4.3 ─── WASM tempfile impl
 │    └── 4.5 ─── WASM time impl
 │
 ├── 1.3 ─── storage/ skeleton
 │    │
 │    ├── 2.1 ─── Storage type aliases (native)
 │    ├── 2.2 ─── Database<K,V> wrapper
 │    │    │
 │    │    └── 2.3 ─── Migrate Index struct
 │    │         │
 │    │         ├── 2.4 ─── Migrate Index methods
 │    │         ├── 2.5 ─── Migrate search subsystem
 │    │         └── 2.6 ─── Migrate update subsystem
 │    │
 │    └── 2.7 ─── MemoryEnv implementation ──── 6.2 (tests)
 │
 ├── 1.4 ─── executor/ skeleton
 │    │
 │    ├── 3.1 ─── Rayon shim
 │    ├── 3.2 ─── WASM Executor
 │    ├── 3.3 ─── Migrate rayon imports
 │    └── 3.4 ─── Grenad feature config
 │
 └── 1.5 ─── CI job (initially failing)
      │
      └───────────────── 6.7 (CI finalized, should pass)

4.2 (mmap migration) ─── depends on 4.1
4.4 (tempfile migration) ─── depends on 4.3
4.6 (time migration) ─── depends on 4.5

5.1–5.6 (conditional compilation) ─── depends on 1.1 (feature flags)

6.1 (WASM check passes) ─── depends on ALL of 2–5
6.3 (wasm-pack test) ─── depends on 6.1 + 2.7
6.4 (gate native tests) ─── depends on 1.1
6.5 (benchmarks) ─── depends on 2.3–2.6 + 3.3
6.6 (binary size) ─── depends on 6.1
```

---

## Effort Estimates

| Epic | Tasks | Estimated Effort |
|------|-------|-----------------|
| **1. Infrastructure** | 1.1–1.5 | 1–2 days |
| **2. Storage abstraction** | 2.1–2.7 | 5–8 days |
| **3. Parallelism abstraction** | 3.1–3.4 | 2–3 days |
| **4. I/O abstractions** | 4.1–4.6 | 2–3 days |
| **5. Conditional compilation** | 5.1–5.6 | 2–3 days |
| **6. Testing & validation** | 6.1–6.7 | 3–4 days |
| **Total** | **27 tasks** | **15–23 days** |

The critical path runs through Epic 2 (storage abstraction), which is the largest and most complex piece. Epics 3, 4, and 5 can be partially parallelized.

---

## Risk Mitigations in Task Sequence

1. **Task 1.1 validates native first** — feature flags must not break existing builds
2. **Tasks 2.1–2.6 are mechanical on native** — type aliases mean zero behavioral change; run tests after each
3. **Task 2.7 (MemoryEnv) is the highest-risk implementation** — write thorough tests (6.2)
4. **Task 3.4 (grenad without rayon/tempfile)** — test early; if grenad doesn't work in sequential mode, need to patch grenad
5. **Task 6.1 is the gate** — everything before it may reveal issues; budget iteration time
6. **Task 6.5 (benchmarks)** — if regression >5%, investigate before proceeding; the type-alias approach should be zero-cost but verify

---

## Post-MVP Follow-up Tasks (not in this plan)

- [ ] Add `wasm-bindgen-rayon` support behind `wasm-threads` feature (REQ-PA-6)
- [ ] Add IndexedDB-backed storage for WASM persistence
- [ ] Port vector store (arroy/hannoy) to WASM with in-memory backend
- [ ] Create `wasm-pack` npm package for browser consumption
- [ ] Add `wasm32-wasip1` target support for edge runtimes
- [ ] Optimize WASM binary size with `wasm-opt` and feature stripping
- [ ] Benchmark search latency on 10k docs in Chrome (REQ-NF-2)
