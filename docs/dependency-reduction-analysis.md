# Dependency Reduction Analysis: wilysearch

## Phase 1: Requirements Analysis

### Current State Assessment

**wilysearch** currently wraps `milli` directly, providing:
- Index management (create, delete, list indexes)
- Document CRUD operations
- Keyword search with filters and ranking
- Hybrid search combining keyword + vector
- Settings management (searchable/filterable/sortable attributes, synonyms, typo tolerance)
- Embedder configurations (OpenAI, Ollama, HuggingFace, REST)

### Current Dependencies

| Dependency | Purpose | Heavy? | Replaceable? |
|------------|---------|--------|--------------|
| `milli` | Core search engine (BM25, indexing, filters) | **Very heavy** (~500K+ LOC) | Yes, partially |
| `meilisearch-types` | Shared types | Medium | Yes, with custom types |
| `http-client` | Embedder HTTP calls | Light | Keep or replace |
| `heed` | LMDB wrapper (via milli) | Medium | Bundled with milli |
| `file-store` | File storage utilities | Light | Keep |

### Milli Feature Usage Analysis

| Feature | Current Usage | Alternative Technology |
|---------|---------------|------------------------|
| **BM25 Full-Text Search** | Core search functionality | Tantivy, sqlite-vec+FTS5, SurrealDB FTS |
| **Typo Tolerance** | Via milli's FST+Levenshtein | SymSpell (query preprocessing) |
| **Synonyms** | Stored in milli index | Query preprocessing layer |
| **Filters** | Via milli's Filter type | SQL WHERE clauses |
| **Ranking** | Via milli's scoring | Custom RRF implementation |
| **Embedders** | Via milli's vector module | Direct embedding API calls |
| **HNSW Vector Index** | Via milli (optional) | LanceDB, sqlite-vec, hnsw_rs, Arroy |

---

## Phase 2: Design - Dependency Reduction Architecture

### Key Insight from Reference Material

The reference text identifies that **typo tolerance and synonyms are query preprocessing steps**, not search engine features. This means:

1. They can be implemented **before** hitting any search backend
2. They're **backend-agnostic** - work with any database
3. They give **more control** than Meilisearch's baked-in approach

### Proposed Architecture Layers

```
User Query: "firball damge resistence"
       │
       ▼
┌──────────────────────────────────────────────────┐
│  Layer 1: Query Preprocessing (NEW)              │
│  ┌─────────────────┐  ┌────────────────────────┐ │
│  │ TypoCorrector   │  │ SynonymExpander        │ │
│  │ (SymSpell)      │  │ (configurable map)     │ │
│  └────────┬────────┘  └───────────┬────────────┘ │
└───────────┼───────────────────────┼──────────────┘
            │                       │
            ▼                       ▼
┌──────────────────────────────────────────────────┐
│  Layer 2: Search Backend (REPLACEABLE)           │
│  ┌─────────────────┐  ┌────────────────────────┐ │
│  │ BM25/FTS Engine │  │ Vector Search          │ │
│  │ Options:        │  │ Options:               │ │
│  │ • milli (heavy) │  │ • VectorStore trait    │ │
│  │ • Tantivy       │  │ • LanceDB              │ │
│  │ • sqlite-vec    │  │ • sqlite-vec           │ │
│  │ • SurrealDB     │  │ • hnsw_rs              │ │
│  └────────┬────────┘  └───────────┬────────────┘ │
└───────────┼───────────────────────┼──────────────┘
            │                       │
            ▼                       ▼
┌──────────────────────────────────────────────────┐
│  Layer 3: Result Fusion (NEW)                    │
│  ┌──────────────────────────────────────────┐   │
│  │ RRF (Reciprocal Rank Fusion)              │   │
│  │ Combines BM25 + Vector scores             │   │
│  └──────────────────────────────────────────┘   │
└──────────────────────────────────────────────────┘
```

### Reduction Scenarios

#### Scenario A: Minimal Change (Keep milli, extract preprocessing)
- **Remove from milli**: Typo tolerance, synonyms
- **Add**: SymSpell + SynonymMap as preprocessing
- **Benefit**: Control over typo/synonym behavior, ~5% dependency reduction
- **Effort**: Low (1-2 days)

#### Scenario B: Replace Vector Search Only
- **Remove from milli**: Vector search (if using built-in HNSW)
- **Add**: sqlite-vec OR hnsw_rs via VectorStore trait
- **Benefit**: Simpler vector management, ~10% dependency reduction
- **Effort**: Low (already have VectorStore trait)

#### Scenario C: Replace milli with Tantivy + Vector DB
- **Remove**: milli entirely
- **Add**: Tantivy (BM25) + LanceDB/sqlite-vec (vectors)
- **Benefit**: ~60% dependency reduction, more control
- **Effort**: High (2-3 weeks)

#### Scenario D: Unified Backend (SurrealDB)
- **Remove**: milli, separate vector store
- **Add**: SurrealDB (BM25 + HNSW in one DB)
- **Benefit**: Single database for everything, simpler architecture
- **Effort**: High (3-4 weeks), maturity concerns, BSL license

#### Scenario E: sqlite-vec + FTS5 (Simplest Migration)
- **Remove**: milli
- **Add**: SQLite with FTS5 extension + sqlite-vec
- **Benefit**: Minimal dependencies, single .db file, well-understood
- **Effort**: Medium (1-2 weeks)

---

## Phase 3: Implementation Tasks

### Recommended Path: Scenario A + B (Incremental)

Start with preprocessing extraction, then optionally replace vector backend.

#### Task 1: Query Preprocessing Layer

```
- [ ] 1. Query Preprocessing Module
  - [ ] 1.1 Add SymSpell dependency and create TypoCorrector
    - Add `symspell = "0.4"` to Cargo.toml
    - Create `src/preprocessing/typo.rs`
    - Implement TypoCorrector with TTRPG corpus support
    - Implement Meilisearch-compatible length rules (5 chars = 1 typo, 9 chars = 2 typos)
    _Requirements: Typo tolerance parity with Meilisearch_

  - [ ] 1.2 Create SynonymMap
    - Create `src/preprocessing/synonyms.rs`
    - Implement multi-way and one-way synonym support
    - Support JSON/TOML configuration loading
    - Generate expanded queries for BM25
    _Requirements: Synonym functionality parity_

  - [ ] 1.3 Create QueryPipeline
    - Create `src/preprocessing/mod.rs`
    - Combine typo correction → synonym expansion → query output
    - Return ProcessedQuery with corrected text and expansion groups
    _Requirements: Clean API for preprocessing_

  - [ ] 1.4 Integrate with existing search
    - Modify `Index::search()` to use QueryPipeline
    - Add option to disable preprocessing per-query
    - Maintain backward compatibility
    _Requirements: Non-breaking API_
```

#### Task 2: Vector Store Improvements (Optional)

```
- [ ] 2. Vector Store Backend Options
  - [ ] 2.1 Implement sqlite-vec VectorStore
    - Add `sqlite-vec` dependency
    - Create `src/vector/sqlite_vec.rs`
    - Implement VectorStore trait for sqlite-vec
    _Requirements: Alternative vector backend_

  - [ ] 2.2 Implement hnsw_rs VectorStore (for larger datasets)
    - Add `hnsw_rs` optional dependency
    - Create `src/vector/hnsw.rs`
    - Implement mmap persistence
    _Requirements: Scalable vector search_
```

#### Task 3: Disable milli Features (Optional)

```
- [ ] 3. Feature-gate milli usage
  - [ ] 3.1 Make milli optional via Cargo feature
    - Restructure to support `default = ["milli-backend"]`
    - Create abstract search backend trait
    _Requirements: Gradual migration path_

  - [ ] 3.2 Implement Tantivy backend (alternative)
    - Add Tantivy as optional dependency
    - Implement SearchBackend trait for Tantivy
    _Requirements: milli-free operation possible_
```

---

## Decision Matrix

| Factor | Keep milli | Tantivy | sqlite-vec+FTS5 | SurrealDB |
|--------|------------|---------|-----------------|-----------|
| **Dependency Size** | Very Large | Medium | Small | Medium |
| **Migration Effort** | None | High | Medium | High |
| **Feature Parity** | 100% | ~90% | ~80% | ~85% |
| **Performance** | Excellent | Excellent | Good (<100K) | Good |
| **Hybrid Search** | Built-in | Manual RRF | Manual RRF | Built-in |
| **Rust Ecosystem** | Meilisearch | Standalone | SQLite | Emerging |
| **License** | MIT | MIT | MIT | BSL 1.1 |

---

## Recommendation

### For Small-to-Medium Datasets (1K-10K documents)

**Recommended: Scenario A + sqlite-vec (Scenario B)**

1. **Phase 1**: Extract typo tolerance and synonyms to preprocessing layer
   - Adds SymSpell (~1K SLoC, O(1) lookups)
   - Moves synonyms to configurable JSON
   - No breaking changes to API

2. **Phase 2**: Add sqlite-vec as VectorStore implementation
   - Already have VectorStore trait
   - sqlite-vec is tiny (<5K SLoC)
   - Works with existing SQLite in TTTRPS

3. **Future**: Evaluate full milli replacement if:
   - Dependency size becomes a problem
   - Need features milli doesn't provide well
   - Performance at scale requires it

### Why NOT full replacement now?

1. **milli works** - The current implementation compiles and passes tests
2. **Effort vs Benefit** - Full replacement is 2-3 weeks for marginal gain at TTTRPS scale
3. **Vector search already abstracted** - VectorStore trait allows backend swapping
4. **Preprocessing gives 80% of benefit** - Typo/synonym control is the main win

---

## Appendix: New Dependencies Summary

If implementing Scenario A + B:

```toml
# Query Preprocessing (NEW)
symspell = "0.5"              # ~1K SLoC, typo correction

# Vector Store Options (NEW, choose one)
sqlite-vec = "0.1"            # ~5K SLoC, embedded vectors
# OR
hnsw_rs = "0.3"               # ~2K SLoC, standalone HNSW

# Keep existing
milli = { path = "../milli" } # Still needed for BM25/indexing
```

Total new code: ~500 lines for preprocessing, ~200 lines for sqlite-vec adapter.
