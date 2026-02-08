# file-store WASM Compatibility — Technical Design

**Version:** 1.0.0
**Date:** 2026-02-07
**Status:** Draft
**Implements:** `Requirements.md` v1.0.0

---

## 1. Overview

The file-store crate is 206 lines of pure filesystem I/O. The design replaces the monolithic `FileStore` struct with an `UpdateStore` trait, a native `FileSystemStore` implementation (wrapping current behavior), and a WASM `MemoryStore` implementation (HashMap-backed). Conditional type aliases provide backward compatibility.

### Design Principles

1. **Minimal disruption:** Type aliases ensure consumer code compiles unchanged on native
2. **Trait-first:** The `UpdateStore` trait is the canonical interface; backends are implementations
3. **Lean:** The crate stays small — no unnecessary abstractions
4. **Uniform data access:** `get_update()` returns `Vec<u8>` on both backends; a native-only `get_update_file()` provides raw `File` handles where needed

---

## 2. Architecture

```
┌──────────────────────────────────────┐
│           file-store crate           │
├──────────────────────────────────────┤
│  pub trait UpdateStore               │
│  ├── new_update() -> (Uuid, Writer)  │
│  ├── get_update(Uuid) -> Vec<u8>     │
│  ├── delete(Uuid)                    │
│  ├── all_uuids() -> Vec<Uuid>        │
│  ├── compute_size(Uuid) -> u64       │
│  └── snapshot(Uuid, &dyn ..)         │
│                                      │
│  pub trait UpdateWriter: Write       │
│  └── persist(self) -> Result<()>     │
├────────────────┬─────────────────────┤
│ Native         │ WASM                │
│ FileSystemStore│ MemoryStore         │
│ ┌────────────┐ │ ┌─────────────────┐ │
│ │ PathBuf    │ │ │ Arc<RwLock<     │ │
│ │ NamedTemp  │ │ │  HashMap<Uuid,  │ │
│ │ fs::File   │ │ │   Vec<u8>>     │ │
│ │ fs::copy   │ │ │ >>             │ │
│ └────────────┘ │ └─────────────────┘ │
│ Writer:        │ Writer:             │
│   File (cur.)  │   MemoryWriter      │
│   NamedTempFile│   Cursor<Vec<u8>>   │
└────────────────┴─────────────────────┘
```

---

## 3. Feature Flag Design

```toml
[features]
default = ["native"]
native = ["dep:tempfile"]
wasm = []

[dependencies]
thiserror = "2.0.17"
tracing = "0.1.41"
uuid = { version = "1.18.1", features = ["serde", "v4"] }
serde = { version = "1.0", features = ["derive"], optional = true }

# Native-only
tempfile = { version = "3.23.0", optional = true }

[target.'cfg(target_arch = "wasm32")'.dependencies]
getrandom = { version = "0.2", features = ["js"] }
```

---

## 4. Module Layout

```
src/
├── lib.rs          # Trait definitions, Error, type aliases, re-exports
├── filesystem.rs   # FileSystemStore + File (native backend)
└── memory.rs       # MemoryStore + MemoryWriter (WASM backend)
```

This replaces the current single `lib.rs` with three files.

---

## 5. Component Designs

### 5.1 Core Trait: `UpdateStore`

**File:** `src/lib.rs`

```rust
use uuid::Uuid;

/// Trait abstracting update file storage.
/// Native: filesystem-backed. WASM: in-memory HashMap.
pub trait UpdateStore: Clone + Send + Sync {
    type Writer: UpdateWriter;

    /// Create a new update with a random UUID.
    fn new_update(&self) -> Result<(Uuid, Self::Writer)>;

    /// Create a new update with a specific UUID.
    fn new_update_with_uuid(&self, uuid: u128) -> Result<(Uuid, Self::Writer)>;

    /// Read the full contents of an update.
    fn get_update(&self, uuid: Uuid) -> Result<Vec<u8>>;

    /// Delete an update.
    fn delete(&self, uuid: Uuid) -> Result<()>;

    /// List all stored update UUIDs.
    fn all_uuids(&self) -> Result<Vec<Uuid>>;

    /// Compute the byte size of a single update.
    fn compute_size(&self, uuid: Uuid) -> Result<u64>;

    /// Compute total byte size of all updates.
    fn compute_total_size(&self) -> Result<u64> {
        let mut total = 0u64;
        for uuid in self.all_uuids()? {
            total += self.compute_size(uuid).unwrap_or(0);
        }
        Ok(total)
    }
}
```

### 5.2 Writer Trait: `UpdateWriter`

```rust
/// A writable handle for a new update.
/// Implements `std::io::Write`. Call `persist()` to finalize.
pub trait UpdateWriter: std::io::Write + Send {
    /// Finalize the write. After this, the data is available via `get_update()`.
    fn persist(self) -> Result<()>;
}
```

### 5.3 Type Aliases (backward compatibility)

```rust
// Compile-time backend selection
#[cfg(feature = "native")]
pub use filesystem::FileSystemStore as Store;
#[cfg(feature = "wasm")]
pub use memory::MemoryStore as Store;

// Backward compatibility aliases
#[cfg(feature = "native")]
pub use filesystem::FileSystemStore as FileStore;
#[cfg(feature = "native")]
pub use filesystem::FileSystemWriter as File;

#[cfg(feature = "native")]
mod filesystem;
#[cfg(feature = "wasm")]
mod memory;
// Always compile both on native for testing
#[cfg(all(feature = "native", test))]
mod memory;
```

### Decision: Trait object vs concrete type aliases

**Context:** Should consumers use `dyn UpdateStore` or concrete types?

**Options:**
1. **Trait object (`Box<dyn UpdateStore>`)** — runtime flexibility, vtable overhead
2. **Concrete type aliases** — zero-cost, compile-time selection
3. **Generic parameter (`Queue<S: UpdateStore>`)** — viral generics in consumers

**Decision:** Option 2 (concrete aliases). The backend is known at compile time. Consumers use `file_store::Store` which resolves to the concrete type. No generics needed, no vtable overhead. The trait exists for test doubles and documentation, but normal usage goes through concrete types.

### 5.4 FileSystemStore (Native)

**File:** `src/filesystem.rs`

```rust
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use uuid::Uuid;

use crate::{Error, Result, UpdateStore, UpdateWriter};

#[derive(Clone, Debug)]
pub struct FileSystemStore {
    path: PathBuf,
}

impl FileSystemStore {
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    /// Native-only: get a raw std::fs::File handle for streaming/mmap.
    pub fn get_update_file(&self, uuid: Uuid) -> Result<fs::File> {
        let path = self.path.join(uuid.to_string());
        Ok(fs::File::open(path)?)
    }

    /// Native-only: get the filesystem path for an update.
    pub fn update_path(&self, uuid: Uuid) -> PathBuf {
        self.path.join(uuid.to_string())
    }

    /// Native-only: copy update to a snapshot directory.
    pub fn snapshot(&self, uuid: Uuid, dst: impl AsRef<Path>) -> Result<()> {
        let src = self.path.join(uuid.to_string());
        let mut dst = dst.as_ref().join("updates/updates_files");
        fs::create_dir_all(&dst)?;
        dst.push(uuid.to_string());
        fs::copy(src, dst)?;
        Ok(())
    }
}

impl UpdateStore for FileSystemStore {
    type Writer = FileSystemWriter;

    fn new_update(&self) -> Result<(Uuid, Self::Writer)> {
        let file = NamedTempFile::new_in(&self.path)?;
        let uuid = Uuid::new_v4();
        let path = self.path.join(uuid.to_string());
        Ok((uuid, FileSystemWriter { file: Some(file), path }))
    }

    fn new_update_with_uuid(&self, uuid: u128) -> Result<(Uuid, Self::Writer)> {
        let file = NamedTempFile::new_in(&self.path)?;
        let uuid = Uuid::from_u128(uuid);
        let path = self.path.join(uuid.to_string());
        Ok((uuid, FileSystemWriter { file: Some(file), path }))
    }

    fn get_update(&self, uuid: Uuid) -> Result<Vec<u8>> {
        let path = self.path.join(uuid.to_string());
        let mut file = fs::File::open(&path).map_err(|e| {
            tracing::error!("Can't access update file {uuid}: {e}");
            e
        })?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        Ok(buf)
    }

    fn delete(&self, uuid: Uuid) -> Result<()> {
        let path = self.path.join(uuid.to_string());
        fs::remove_file(&path).map_err(|e| {
            tracing::error!("Can't delete file {uuid}: {e}");
            e
        })?;
        Ok(())
    }

    fn all_uuids(&self) -> Result<Vec<Uuid>> {
        let mut uuids = Vec::new();
        for entry in self.path.read_dir()? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().ok_or(Error::CouldNotParseFileNameAsUtf8)?;
            if !name.starts_with('.') {
                uuids.push(Uuid::from_str(name)?);
            }
        }
        Ok(uuids)
    }

    fn compute_size(&self, uuid: Uuid) -> Result<u64> {
        let file = self.get_update_file(uuid)?;
        Ok(file.metadata()?.len())
    }
}
```

### FileSystemWriter (native File replacement)

```rust
pub struct FileSystemWriter {
    path: PathBuf,
    file: Option<NamedTempFile>,
}

impl FileSystemWriter {
    /// Backward-compat: construct from parts.
    pub fn from_parts(path: PathBuf, file: Option<NamedTempFile>) -> Self {
        Self { path, file }
    }

    /// Backward-compat: decompose into parts.
    pub fn into_parts(self) -> (PathBuf, Option<NamedTempFile>) {
        (self.path, self.file)
    }

    /// Backward-compat: create a no-op writer.
    pub fn dry_file() -> Result<Self> {
        Ok(Self { path: PathBuf::new(), file: None })
    }

    /// Backward-compat: persist and return raw File handle.
    pub fn persist_to_file(self) -> Result<Option<fs::File>> {
        let Some(file) = self.file else { return Ok(None) };
        Ok(Some(file.persist(&self.path)?))
    }
}

impl UpdateWriter for FileSystemWriter {
    fn persist(self) -> Result<()> {
        if let Some(file) = self.file {
            file.persist(&self.path)?;
        }
        Ok(())
    }
}

impl Write for FileSystemWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.file.as_mut() {
            Some(file) => file.write(buf),
            None => Ok(buf.len()), // dry file: discard
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self.file.as_mut() {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}
```

### 5.5 MemoryStore (WASM)

**File:** `src/memory.rs`

```rust
use std::collections::HashMap;
use std::io::{Cursor, Write};
use std::sync::{Arc, RwLock};
use uuid::Uuid;

use crate::{Error, Result, UpdateStore, UpdateWriter};

/// In-memory update store backed by HashMap<Uuid, Vec<u8>>.
#[derive(Clone, Debug)]
pub struct MemoryStore {
    inner: Arc<RwLock<HashMap<Uuid, Vec<u8>>>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self { inner: Arc::new(RwLock::new(HashMap::new())) }
    }

    /// Serialize the entire store contents for transfer.
    #[cfg(feature = "serde")]
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let guard = self.inner.read().map_err(|_| Error::LockPoisoned)?;
        bincode::serialize(&*guard).map_err(|e| Error::Serialization(e.to_string()))
    }

    /// Deserialize from bytes.
    #[cfg(feature = "serde")]
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let map: HashMap<Uuid, Vec<u8>> =
            bincode::deserialize(data).map_err(|e| Error::Serialization(e.to_string()))?;
        Ok(Self { inner: Arc::new(RwLock::new(map)) })
    }
}

impl Default for MemoryStore {
    fn default() -> Self { Self::new() }
}

impl UpdateStore for MemoryStore {
    type Writer = MemoryWriter;

    fn new_update(&self) -> Result<(Uuid, Self::Writer)> {
        let uuid = Uuid::new_v4();
        Ok((uuid, MemoryWriter {
            uuid,
            buffer: Cursor::new(Vec::new()),
            store: self.inner.clone(),
        }))
    }

    fn new_update_with_uuid(&self, uuid: u128) -> Result<(Uuid, Self::Writer)> {
        let uuid = Uuid::from_u128(uuid);
        Ok((uuid, MemoryWriter {
            uuid,
            buffer: Cursor::new(Vec::new()),
            store: self.inner.clone(),
        }))
    }

    fn get_update(&self, uuid: Uuid) -> Result<Vec<u8>> {
        let guard = self.inner.read().map_err(|_| Error::LockPoisoned)?;
        guard.get(&uuid).cloned().ok_or_else(|| {
            tracing::error!("Can't access update {uuid}: not found in memory store");
            Error::NotFound(uuid)
        })
    }

    fn delete(&self, uuid: Uuid) -> Result<()> {
        let mut guard = self.inner.write().map_err(|_| Error::LockPoisoned)?;
        if guard.remove(&uuid).is_none() {
            tracing::error!("Can't delete update {uuid}: not found in memory store");
            return Err(Error::NotFound(uuid));
        }
        Ok(())
    }

    fn all_uuids(&self) -> Result<Vec<Uuid>> {
        let guard = self.inner.read().map_err(|_| Error::LockPoisoned)?;
        Ok(guard.keys().copied().collect())
    }

    fn compute_size(&self, uuid: Uuid) -> Result<u64> {
        let guard = self.inner.read().map_err(|_| Error::LockPoisoned)?;
        guard.get(&uuid)
            .map(|v| v.len() as u64)
            .ok_or(Error::NotFound(uuid))
    }
}

/// In-memory writer that buffers data and inserts into the store on persist.
pub struct MemoryWriter {
    uuid: Uuid,
    buffer: Cursor<Vec<u8>>,
    store: Arc<RwLock<HashMap<Uuid, Vec<u8>>>>,
}

impl UpdateWriter for MemoryWriter {
    fn persist(self) -> Result<()> {
        let data = self.buffer.into_inner();
        let mut guard = self.store.write().map_err(|_| Error::LockPoisoned)?;
        guard.insert(self.uuid, data);
        Ok(())
    }
}

impl Write for MemoryWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buffer.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.buffer.flush()
    }
}
```

### 5.6 Error Type

```rust
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Could not parse file name as utf-8")]
    CouldNotParseFileNameAsUtf8,

    #[error(transparent)]
    IoError(#[from] std::io::Error),

    #[cfg(feature = "native")]
    #[error(transparent)]
    PersistError(#[from] tempfile::PersistError),

    #[error(transparent)]
    UuidError(#[from] uuid::Error),

    #[error("Update not found: {0}")]
    NotFound(Uuid),

    #[error("Lock poisoned")]
    LockPoisoned,

    #[error("Serialization error: {0}")]
    Serialization(String),
}
```

The `ErrorCode` impl in `meilisearch-types` adds:

```rust
impl ErrorCode for file_store::Error {
    fn error_code(&self) -> Code {
        match self {
            Self::IoError(e) => e.error_code(),
            #[cfg(feature = "native")]
            Self::PersistError(e) => e.error_code(),
            Self::NotFound(_) => Code::Internal,
            Self::LockPoisoned => Code::Internal,
            Self::Serialization(_) => Code::Internal,
            Self::CouldNotParseFileNameAsUtf8 | Self::UuidError(_) => Code::Internal,
        }
    }
}
```

---

## 6. Consumer Impact Analysis

### index-scheduler/queue/mod.rs

**Current:**
```rust
pub(crate) file_store: FileStore,
// ...
FileStore::new(&options.update_file_path)?
```

**After (unchanged on native):**
```rust
pub(crate) file_store: FileStore,  // FileStore is alias to FileSystemStore
// ...
FileStore::new(&options.update_file_path)?  // Same constructor
```

All consumer code compiles unchanged because `FileStore` is a type alias.

### meilisearch/routes/indexes/documents.rs

**Current uses `File::into_parts()` and `File::from_parts()`:**
```rust
let (path, file) = update_file.into_parts();
// ... manipulate NamedTempFile ...
let update_file = file_store::File::from_parts(path, file);
```

These are native-only patterns. They remain available through `FileSystemWriter::into_parts()` and `FileSystemWriter::from_parts()`, and `file_store::File` is aliased to `FileSystemWriter`.

### process_index_operation.rs

**Current:**
```rust
let content_file = self.queue.file_store.get_update(*content_uuid)?;
// content_file is std::fs::File, passed to milli
```

**After:** `get_update()` returns `Vec<u8>`. For native paths that need a `std::fs::File`, use `get_update_file()`:

```rust
// Option A: Use Vec<u8> (works on both backends)
let content_data = self.queue.file_store.get_update(*content_uuid)?;

// Option B: Native-only raw file handle (for mmap/streaming)
let content_file = self.queue.file_store.get_update_file(*content_uuid)?;
```

This is the **one breaking change** on native — `get_update()` return type changes from `std::fs::File` to `Vec<u8>`. The `get_update_file()` escape hatch preserves the old behavior.

### Decision: get_update return type

**Context:** Should `get_update()` return `Vec<u8>` (uniform) or keep returning `std::fs::File` on native?

**Options:**
1. `Vec<u8>` everywhere — simple trait, but reads entire file into memory on native
2. `Box<dyn Read>` — avoids full read but requires trait object
3. Keep `std::fs::File` on native, `Vec<u8>` on WASM — breaks uniform trait
4. Return `Vec<u8>` on trait, add `get_update_file()` native-only escape hatch

**Decision:** Option 4. The trait returns `Vec<u8>` for uniformity. Native code that needs streaming uses the concrete `FileSystemStore::get_update_file()` method. This keeps the trait clean and the native escape hatch explicit.

---

## 7. Testing Strategy

### Shared Test Suite

Both backends are tested with the same logical tests via a helper:

```rust
fn test_update_store(store: &impl UpdateStore) {
    // Write
    let (uuid, mut writer) = store.new_update().unwrap();
    writer.write_all(b"Hello world").unwrap();
    writer.persist().unwrap();

    // Read
    let data = store.get_update(uuid).unwrap();
    assert_eq!(data, b"Hello world");

    // Size
    assert_eq!(store.compute_size(uuid).unwrap(), 11);

    // List
    let uuids = store.all_uuids().unwrap();
    assert!(uuids.contains(&uuid));

    // Delete
    store.delete(uuid).unwrap();
    assert!(store.get_update(uuid).is_err());
}

#[test]
#[cfg(feature = "native")]
fn test_filesystem_store() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = FileSystemStore::new(dir.path()).unwrap();
    test_update_store(&store);
}

#[test]
fn test_memory_store() {
    let store = MemoryStore::new();
    test_update_store(&store);
}
```

---

## 8. Decisions Log

| # | Decision | Rationale |
|---|----------|-----------|
| D-1 | Concrete type aliases over `dyn UpdateStore` | Zero-cost; backend known at compile time |
| D-2 | `get_update()` returns `Vec<u8>` with native escape hatch | Uniform trait; streaming available via `get_update_file()` |
| D-3 | `all_uuids()` returns `Vec<Uuid>` not `Iterator` | Simpler trait; iterator lifetimes complex across backends |
| D-4 | `HashMap` not `BTreeMap` for MemoryStore | Order not required; faster lookup for primary use case |
| D-5 | `Arc<RwLock<HashMap>>` for Clone + thread safety | Matches FileStore's Clone semantics |
| D-6 | Split into 3 files (lib.rs, filesystem.rs, memory.rs) | Clean separation; each backend is self-contained |
| D-7 | `PersistError` cfg-gated | `tempfile::PersistError` unavailable on WASM |
| D-8 | Memory backend compiled in native test cfg | Allows testing MemoryStore on native without wasm target |
