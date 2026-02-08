# file-store WASM Compatibility — Implementation Tasks

**Version:** 1.0.0
**Date:** 2026-02-07
**Status:** Draft
**Implements:** `Design.md` v1.0.0
**Traceability:** Each task references Requirements (REQ-*) and Design decisions (D-*)

---

## Task Sequencing: Foundation-First

The file-store crate is small (206 lines), so the refactoring is compact. Sequence:

1. **Restructure** — split lib.rs into modules, add feature flags
2. **Trait definition** — define `UpdateStore` and `UpdateWriter`
3. **Native backend** — move current code into `FileSystemStore`
4. **WASM backend** — implement `MemoryStore`
5. **Error adaptation** — update Error enum and ErrorCode impl
6. **Backward compatibility** — type aliases, consumer verification
7. **Testing** — shared test suite for both backends

---

## Epic 1: Crate Restructuring

### Task 1.1: Add feature flags to Cargo.toml
- Add `native` (default) and `wasm` features
- Make `tempfile` optional behind `native`
- Add `serde` as optional (for MemoryStore serialization)
- Add `getrandom` with `js` feature for WASM target
- Verify `cargo check --features native` passes
- **Files:** `crates/file-store/Cargo.toml`
- _Requirements: REQ-FF-1, REQ-FF-2, REQ-FF-3_

```toml
[features]
default = ["native"]
native = ["dep:tempfile"]
wasm = []

[dependencies]
thiserror = "2.0.17"
tracing = "0.1.41"
uuid = { version = "1.18.1", features = ["serde", "v4"] }
tempfile = { version = "3.23.0", optional = true }

[target.'cfg(target_arch = "wasm32")'.dependencies]
getrandom = { version = "0.2", features = ["js"] }
```

### Task 1.2: Create module structure
- Create `src/filesystem.rs` (empty, will hold native backend)
- Create `src/memory.rs` (empty, will hold WASM backend)
- Update `src/lib.rs` to declare modules conditionally:
  ```rust
  #[cfg(feature = "native")]
  mod filesystem;
  #[cfg(any(feature = "wasm", test))]
  mod memory;
  ```
- Verify compilation unchanged
- **Files:** `src/lib.rs`, `src/filesystem.rs`, `src/memory.rs`
- _Requirements: Design D-6_

---

## Epic 2: Trait Definitions

### Task 2.1: Define `UpdateStore` trait
- Add to `src/lib.rs`:
  - `pub trait UpdateStore: Clone + Send + Sync`
  - Associated type `Writer: UpdateWriter`
  - Methods: `new_update`, `new_update_with_uuid`, `get_update`, `delete`, `all_uuids`, `compute_size`, `compute_total_size` (with default impl)
- Per Design Section 5.1
- **Files:** `src/lib.rs`
- _Requirements: REQ-TR-1, REQ-TR-2_

### Task 2.2: Define `UpdateWriter` trait
- Add to `src/lib.rs`:
  - `pub trait UpdateWriter: std::io::Write + Send`
  - Method: `fn persist(self) -> Result<()>`
- Per Design Section 5.2
- **Files:** `src/lib.rs`
- _Requirements: REQ-TR-2, REQ-TR-3_

---

## Epic 3: Native Backend (FileSystemStore)

### Task 3.1: Move `FileStore` code to `FileSystemStore` in `filesystem.rs`
- Move the existing `FileStore` struct to `src/filesystem.rs` renamed as `FileSystemStore`
- Move the existing `File` struct to `src/filesystem.rs` renamed as `FileSystemWriter`
- Implement `UpdateStore for FileSystemStore`
- Implement `UpdateWriter for FileSystemWriter`
- Keep all native-only methods (`get_update_file`, `update_path`, `snapshot`)
- Keep backward-compat methods on `FileSystemWriter` (`from_parts`, `into_parts`, `dry_file`, `persist_to_file`)
- **Files:** `src/filesystem.rs`
- _Requirements: REQ-FS-1, REQ-FS-2, REQ-FS-3, REQ-FS-4_

### Task 3.2: Change `get_update` return type
- `UpdateStore::get_update()` returns `Vec<u8>` (reads file contents)
- Add `FileSystemStore::get_update_file()` returning `std::fs::File` for native consumers that need raw file handles
- This is the one signature change from the current API
- **Files:** `src/filesystem.rs`
- _Requirements: REQ-FS-3, REQ-FS-4, Design D-2_

### Task 3.3: Change `all_uuids` return type
- Current: returns `impl Iterator<Item = Result<Uuid>>`
- New: returns `Result<Vec<Uuid>>` (collects eagerly)
- Simpler trait signature; the iterator was always collected by consumers anyway
- **Files:** `src/filesystem.rs`
- _Requirements: Design D-3_

### Task 3.4: Add type aliases for backward compatibility
- In `src/lib.rs`:
  ```rust
  #[cfg(feature = "native")]
  pub use filesystem::FileSystemStore as FileStore;
  #[cfg(feature = "native")]
  pub use filesystem::FileSystemWriter as File;
  #[cfg(feature = "native")]
  pub use filesystem::FileSystemStore as Store;
  #[cfg(feature = "wasm")]
  pub use memory::MemoryStore as Store;
  ```
- **Files:** `src/lib.rs`
- _Requirements: REQ-BC-1, REQ-BC-2, REQ-FF-5, Design D-1_

---

## Epic 4: WASM Backend (MemoryStore)

### Task 4.1: Implement `MemoryStore`
- `HashMap<Uuid, Vec<u8>>` wrapped in `Arc<RwLock<...>>`
- Implement `Clone`, `Debug`, `Default`
- Implement all `UpdateStore` trait methods per Design Section 5.5
- `new()` takes no arguments
- **Files:** `src/memory.rs`
- _Requirements: REQ-MS-1, REQ-MS-2, REQ-MS-6_

### Task 4.2: Implement `MemoryWriter`
- Wraps `Cursor<Vec<u8>>` + `Arc<RwLock<HashMap<Uuid, Vec<u8>>>>` + UUID
- `impl Write` delegates to cursor
- `persist()` moves buffer contents into the store's HashMap
- **Files:** `src/memory.rs`
- _Requirements: REQ-MS-3, REQ-MS-4_

### Task 4.3: Add serialization support
- `to_bytes()` / `from_bytes()` on `MemoryStore` using bincode
- Gate behind `#[cfg(feature = "serde")]` or always-on (bincode is lightweight)
- **Files:** `src/memory.rs`
- _Requirements: REQ-MS-5_

---

## Epic 5: Error Handling

### Task 5.1: Update Error enum
- Gate `PersistError` behind `#[cfg(feature = "native")]`
- Add `NotFound(Uuid)` variant
- Add `LockPoisoned` variant
- Add `Serialization(String)` variant
- **Files:** `src/lib.rs`
- _Requirements: REQ-ER-1, REQ-ER-2, REQ-ER-3_

### Task 5.2: Update ErrorCode impl in meilisearch-types
- Add match arms for new variants (`NotFound`, `LockPoisoned`, `Serialization`)
- Gate `PersistError` arm behind `#[cfg(feature = "native")]`
- **Files:** `crates/meilisearch-types/src/error.rs`
- _Requirements: REQ-ER-4_

---

## Epic 6: Consumer Adaptation

### Task 6.1: Update index-scheduler/queue/mod.rs
- `file_store: FileStore` field — unchanged (type alias)
- `FileStore::new(&path)` call — unchanged
- `file_store.get_update(uuid)` — now returns `Vec<u8>` instead of `std::fs::File`
  - If the consumer needs `std::fs::File`, use `file_store.get_update_file(uuid)` (native-only)
  - Check `process_index_operation.rs`, `process_dump_creation.rs` — they pass the File to milli
  - These need to use `get_update_file()` on native to preserve streaming behavior
- `file_store.all_uuids()` — now returns `Result<Vec<Uuid>>` not `Result<impl Iterator>`
  - Update call sites that chained `.collect()` — they can remove the collect
- `file_store.compute_total_size()` — unchanged
- **Files:** `crates/index-scheduler/src/queue/mod.rs`, `src/scheduler/process_index_operation.rs`, `src/scheduler/process_dump_creation.rs`, `src/scheduler/create_batch.rs`, `src/scheduler/process_snapshot_creation.rs`, `src/utils.rs`, `src/insta_snapshot.rs`
- _Requirements: REQ-BC-3, REQ-BC-4_

### Task 6.2: Update meilisearch/routes/indexes/documents.rs
- `File::from_parts()` and `File::into_parts()` — available via `file_store::File` alias
- `File::persist()` — currently returns `Option<StdFile>`; the `UpdateWriter::persist()` returns `Result<()>`. The native `FileSystemWriter` keeps `persist_to_file()` for backward compat
- Verify the NDJSON path that manipulates `NamedTempFile` internals still works
- **Files:** `crates/meilisearch/src/routes/indexes/documents.rs`
- _Requirements: REQ-BC-3_

### Task 6.3: Update meilitool
- `FileStore::new()`, `get_update()`, `all_uuids()`, `update_path()` — all available via aliases
- `get_update()` return type change: adapt if needed
- **Files:** `crates/meilitool/src/main.rs`, `crates/meilitool/src/upgrade/v1_12.rs`
- _Requirements: REQ-BC-4_

### Task 6.4: Update meilisearch/error.rs
- `MeilisearchHttpError::FileStore` variant — unchanged, `file_store::Error` still exists
- **Files:** `crates/meilisearch/src/error.rs`
- _Requirements: REQ-BC-4_

---

## Epic 7: Testing & Validation

### Task 7.1: Write shared `test_update_store` helper
- Generic function testing write-persist-read round-trip, size, list, delete
- Per Design Section 7
- **Files:** `src/lib.rs` (test module)
- _Requirements: REQ-TS-3, REQ-TS-4, REQ-TS-5_

### Task 7.2: Write FileSystemStore tests
- Adapt existing `all_uuids` test to use `FileSystemStore`
- Add `test_update_store(&filesystem_store)` call
- Run with `--features native`
- **Files:** `src/filesystem.rs` or `src/lib.rs` (test module)
- _Requirements: REQ-TS-1, REQ-TS-3_

### Task 7.3: Write MemoryStore tests
- `test_update_store(&memory_store)` call
- Test serialization round-trip (`to_bytes` / `from_bytes`)
- Test concurrent access (multiple clones writing/reading)
- Test `NotFound` error on missing UUID
- **Files:** `src/memory.rs` (test module)
- _Requirements: REQ-TS-2, REQ-TS-3, REQ-TS-4, REQ-TS-5, REQ-TS-6_

### Task 7.4: Verify WASM compilation
- `cargo check --target wasm32-unknown-unknown --features wasm --no-default-features -p file-store`
- Fix any remaining issues
- **Files:** Various
- _Requirements: REQ-FF-4_

### Task 7.5: Verify native consumer compilation
- `cargo check --features native` for workspace
- Ensure all consumer crates (index-scheduler, meilisearch, meilitool, meilisearch-types) compile
- Run full test suite
- **Files:** None (verification only)
- _Requirements: REQ-BC-3, REQ-BC-4_

---

## Dependency Graph

```
1.1 (Cargo.toml) ──┐
                    ├── 1.2 (module structure)
                    │    │
                    │    ├── 2.1 (UpdateStore trait)
                    │    └── 2.2 (UpdateWriter trait)
                    │         │
                    │    ┌────┴────────────────────┐
                    │    │                         │
                    │    ▼                         ▼
                    │   3.1 (FileSystemStore)     4.1 (MemoryStore)
                    │   3.2 (get_update Vec<u8>)  4.2 (MemoryWriter)
                    │   3.3 (all_uuids Vec)       4.3 (serialization)
                    │    │                         │
                    │    └────────┬────────────────┘
                    │             │
                    │    3.4 (type aliases) ◄── 5.1 (Error update)
                    │             │                   │
                    │             ▼                   ▼
                    │    6.1 (index-scheduler)   5.2 (ErrorCode)
                    │    6.2 (documents route)
                    │    6.3 (meilitool)
                    │    6.4 (error.rs)
                    │             │
                    │             ▼
                    └── 7.1–7.5 (testing & validation)
```

Epics 3 and 4 can proceed **in parallel** since they're independent implementations of the same trait.

---

## Effort Estimates

| Epic | Tasks | Estimated Effort |
|------|-------|-----------------|
| **1. Restructuring** | 1.1–1.2 | 30 min |
| **2. Trait definitions** | 2.1–2.2 | 30 min |
| **3. Native backend** | 3.1–3.4 | 2 hours |
| **4. WASM backend** | 4.1–4.3 | 1.5 hours |
| **5. Error handling** | 5.1–5.2 | 30 min |
| **6. Consumer adaptation** | 6.1–6.4 | 2 hours |
| **7. Testing** | 7.1–7.5 | 1.5 hours |
| **Total** | **19 tasks** | **~8 hours** |

This is a one-day refactoring. The crate is small and well-contained.

---

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| `get_update()` return type change breaks consumers | High | Medium | `get_update_file()` escape hatch; update consumers in Task 6.1 |
| `all_uuids()` return type change breaks consumers | Medium | Low | Consumers already `.collect()` the iterator |
| Documents route `into_parts()`/`from_parts()` breaks | Low | High | Preserved on `FileSystemWriter`; `File` alias maps to it |
| MemoryStore lock contention | Low | Low | Single-writer pattern; WASM is single-threaded anyway |
| `uuid` crate `getrandom` fails on WASM | Low | High | `getrandom` with `js` feature is proven |

---

## Post-MVP Follow-up

- [ ] IndexedDB-backed `PersistentMemoryStore` for WASM browser persistence
- [ ] Streaming `UpdateWriter` for WASM (chunked writes via ReadableStream)
- [ ] Make `index-scheduler` generic over `UpdateStore` for full WASM support
- [ ] Add `snapshot()` to `UpdateStore` trait for cross-backend snapshots
