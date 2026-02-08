# file-store WASM Compatibility — Requirements Specification

**Version:** 1.0.0
**Date:** 2026-02-07
**Status:** Draft
**Crate:** `file-store` v1.35.0 (update file storage for Meilisearch)

---

## 1. Problem Statement

The `file-store` crate is a thin wrapper around filesystem operations that stores update files (document payloads) associated with Meilisearch tasks. Every function performs direct `std::fs` I/O — creating directories, writing temp files, copying, deleting, reading directory entries. None of this is available on `wasm32-unknown-unknown`. The crate must be abstracted behind an `UpdateStore` trait to allow an in-memory `HashMap<Uuid, Vec<u8>>` backend for WASM while preserving the existing filesystem backend for native.

## 2. Crate Anatomy

### Source

Single file: `src/lib.rs` (206 lines, including tests)

### Public API Surface

| Type | Members | fs Operations |
|------|---------|---------------|
| `FileStore` | `new(path)` | `fs::create_dir_all` |
| | `new_update() -> (Uuid, File)` | `NamedTempFile::new_in` |
| | `new_update_with_uuid(u128) -> (Uuid, File)` | `NamedTempFile::new_in` |
| | `get_update(Uuid) -> StdFile` | `File::open` |
| | `update_path(Uuid) -> PathBuf` | Pure path construction |
| | `snapshot(Uuid, dst)` | `fs::create_dir_all`, `fs::copy` |
| | `compute_total_size() -> u64` | Iterates `all_uuids`, calls `metadata().len()` |
| | `compute_size(Uuid) -> u64` | `File::open` + `metadata().len()` |
| | `delete(Uuid)` | `fs::remove_file` |
| | `all_uuids() -> Iterator<Uuid>` | `read_dir`, filename parsing |
| `File` | `from_parts(PathBuf, Option<NamedTempFile>)` | None (constructor) |
| | `into_parts() -> (PathBuf, Option<NamedTempFile>)` | None (destructor) |
| | `dry_file() -> File` | None (empty sentinel) |
| | `persist() -> Option<StdFile>` | `NamedTempFile::persist` |
| | `impl Write` | Delegates to `NamedTempFile::write` |
| `Error` | `CouldNotParseFileNameAsUtf8` | — |
| | `IoError(std::io::Error)` | — |
| | `PersistError(tempfile::PersistError)` | — |
| | `UuidError(uuid::Error)` | — |

### Dependencies

| Dependency | WASM Compatible | Used For |
|------------|-----------------|----------|
| `tempfile` | NO | `NamedTempFile`, `PersistError` |
| `thiserror` | YES | Error derive |
| `tracing` | YES | Logging |
| `uuid` | YES (with `js` feature for getrandom) | File identification |

### Consumers (7 crates)

| Consumer | Usage Pattern |
|----------|---------------|
| `index-scheduler/queue` | Holds `FileStore` field; calls `new_update`, `get_update`, `delete`, `all_uuids`, `compute_total_size` |
| `index-scheduler/process_index_operation` | `file_store.get_update(uuid)` → `StdFile` → passed to milli |
| `index-scheduler/create_batch` | `file_store.compute_size(uuid)` |
| `index-scheduler/process_dump_creation` | `file_store.get_update(uuid)` |
| `index-scheduler/process_snapshot_creation` | `file_store.update_path(uuid)` → `fs::copy` |
| `meilisearch/routes/indexes/documents` | `File::from_parts`, `File::into_parts`, `File::persist`, `impl Write` |
| `meilitool` | `FileStore::new`, `get_update`, `all_uuids`, `update_path` |
| `meilisearch-types/error` | `impl ErrorCode for file_store::Error` |

## 3. Scope

### In Scope

- Define `UpdateStore` trait abstracting all file-store operations
- Implement `FileSystemStore` (wraps current `FileStore`, zero behavioral change)
- Implement `MemoryStore` (in-memory `HashMap<Uuid, Vec<u8>>` for WASM)
- Define `UpdateWriter` trait abstracting `File` (the writable update handle)
- Feature-flag the native/WASM backends
- Adapt error types for WASM (no `PersistError` in memory backend)
- Preserve all existing consumer code compatibility on native

### Out of Scope

- Modifying consumer crates (index-scheduler, meilisearch routes) — they continue using `file-store` types
- IndexedDB persistence for WASM (future enhancement)
- Streaming writes for large documents in WASM (future enhancement)

## 4. User Stories

**US-1:** As a WASM runtime consumer, I want to store update files in memory so that document indexing works without filesystem access.

**US-2:** As a Meilisearch maintainer, I want the existing filesystem behavior unchanged behind a feature flag so that native builds are unaffected.

**US-3:** As a consumer crate author, I want the public API surface to remain backward-compatible so I don't need to modify my code when using the native backend.

**US-4:** As a developer, I want the in-memory store to support serialization so that update file data can be transferred to/from WASM runtimes.

## 5. Requirements (EARS Format)

### 5.1 Trait Abstraction

**REQ-TR-1:** The crate SHALL define an `UpdateStore` trait that captures all operations currently on `FileStore`:
- `new_update() -> Result<(Uuid, Self::Writer)>`
- `new_update_with_uuid(u128) -> Result<(Uuid, Self::Writer)>`
- `get_update(Uuid) -> Result<Vec<u8>>`
- `delete(Uuid) -> Result<()>`
- `all_uuids() -> Result<Vec<Uuid>>`
- `compute_size(Uuid) -> Result<u64>`
- `compute_total_size() -> Result<u64>`
- `snapshot(Uuid, &dyn UpdateStore) -> Result<()>`

**REQ-TR-2:** The trait SHALL have an associated type `Writer: std::io::Write` representing the writable handle for new updates.

**REQ-TR-3:** The `Writer` type SHALL support a `persist()` method that finalizes the write and makes it available via `get_update()`.

**REQ-TR-4:** `snapshot()` SHALL copy an update from one store to another (same or different backend), replacing the current `snapshot(uuid, path)` signature to be backend-agnostic.

### 5.2 Feature Flags

**REQ-FF-1:** The crate SHALL define `native` (default) and `wasm` feature flags.

**REQ-FF-2:** WHEN `native` is enabled THEN `tempfile` SHALL be a dependency and `FileSystemStore` SHALL be available.

**REQ-FF-3:** WHEN `wasm` is enabled THEN `tempfile` SHALL NOT be a dependency.

**REQ-FF-4:** WHEN `wasm` is enabled THEN `cargo check --target wasm32-unknown-unknown` SHALL succeed.

**REQ-FF-5:** A type alias `Store` SHALL resolve to `FileSystemStore` on native and `MemoryStore` on WASM, so consumers can use `file_store::Store` without cfg-gating.

### 5.3 FileSystemStore (Native Backend)

**REQ-FS-1:** `FileSystemStore` SHALL wrap the current `FileStore` implementation with zero behavioral change.

**REQ-FS-2:** `FileSystemStore::Writer` SHALL be the current `File` struct (wrapping `NamedTempFile`).

**REQ-FS-3:** `get_update()` SHALL return `Vec<u8>` (reading the file contents) instead of `std::fs::File`, to provide a uniform return type across backends.

**REQ-FS-4:** IF a consumer needs a raw `std::fs::File` handle (e.g., for mmap or streaming) THEN a native-only `get_update_file(Uuid) -> Result<std::fs::File>` method SHALL be available behind `#[cfg(feature = "native")]`.

### 5.4 MemoryStore (WASM Backend)

**REQ-MS-1:** `MemoryStore` SHALL store update data in `HashMap<Uuid, Vec<u8>>` (or `BTreeMap` for deterministic ordering).

**REQ-MS-2:** `MemoryStore` SHALL be `Clone` (using `Arc<RwLock<...>>` internally).

**REQ-MS-3:** `MemoryStore::Writer` SHALL be a struct wrapping `Cursor<Vec<u8>>` that implements `std::io::Write`.

**REQ-MS-4:** `Writer::persist()` SHALL move the buffered bytes into the `MemoryStore`'s map under the assigned UUID.

**REQ-MS-5:** `MemoryStore` SHALL support serialization to/from bytes via serde so that stored updates can be transferred across runtime boundaries.

**REQ-MS-6:** `MemoryStore::new()` SHALL NOT require any path argument.

### 5.5 Error Handling

**REQ-ER-1:** The `Error` enum SHALL remain backward-compatible on native (same variants).

**REQ-ER-2:** The `PersistError` variant SHALL be gated behind `#[cfg(feature = "native")]` since `tempfile::PersistError` doesn't exist in WASM.

**REQ-ER-3:** A new `NotFound` variant SHALL be added for when a UUID doesn't exist in the memory store.

**REQ-ER-4:** The `ErrorCode` impl in `meilisearch-types` SHALL handle the new variant.

### 5.6 Backward Compatibility

**REQ-BC-1:** The existing type name `FileStore` SHALL remain available as a type alias to `FileSystemStore` on native.

**REQ-BC-2:** The existing type name `File` (the writer) SHALL remain available on native.

**REQ-BC-3:** All existing method signatures on `FileStore` SHALL continue to work unchanged when `native` feature is active.

**REQ-BC-4:** Consumer crates that reference `file_store::FileStore`, `file_store::File`, and `file_store::Error` SHALL compile without modification on native.

### 5.7 Testing

**REQ-TS-1:** The existing `all_uuids` test SHALL pass unchanged on native.

**REQ-TS-2:** An equivalent `all_uuids` test SHALL pass for `MemoryStore`.

**REQ-TS-3:** Write-persist-read round-trip SHALL be tested for both backends.

**REQ-TS-4:** Delete and `all_uuids` consistency SHALL be tested for both backends.

**REQ-TS-5:** `compute_total_size` and `compute_size` SHALL be tested for both backends.

**REQ-TS-6:** `MemoryStore` serialization round-trip SHALL be tested.

## 6. Constraints

**C-1:** The crate is 206 lines. The refactoring must not bloat it beyond necessity — keep it lean.

**C-2:** `meilisearch-types/src/error.rs` has `impl ErrorCode for file_store::Error` — the Error type must remain compatible.

**C-3:** The documents route (`meilisearch/routes/indexes/documents.rs`) calls `File::into_parts()` and `File::from_parts()` which expose `NamedTempFile` internals. These must remain available on native but can be gated.

**C-4:** `process_index_operation.rs` passes the result of `get_update()` to milli's document indexer which expects readable data. On native it currently passes `std::fs::File`; the trait must provide data that milli can consume.

## 7. Acceptance Criteria

1. `cargo check --features native` passes (zero regression)
2. `cargo check --target wasm32-unknown-unknown --features wasm` passes
3. All existing tests pass with `--features native`
4. MemoryStore round-trip test passes in `wasm-pack test --headless --chrome`
5. Consumer crates compile unchanged on native
