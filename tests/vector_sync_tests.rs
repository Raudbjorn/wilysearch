//! Integration tests for vector store synchronization during document operations.
//!
//! Verifies that `_vectors` data flows from documents into the external VectorStore
//! during add/update, and that deletions and clears propagate correctly.

use serde_json::json;
use std::sync::Arc;
use tempfile::TempDir;
use wilysearch::core::{InMemoryVectorStore, Meilisearch, MeilisearchOptions};

/// Create a Meilisearch instance with an InMemoryVectorStore attached.
fn setup() -> (Meilisearch, Arc<InMemoryVectorStore>, TempDir) {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let store = Arc::new(InMemoryVectorStore::new());
    let options = MeilisearchOptions {
        db_path: temp_dir.path().to_path_buf(),
        max_index_size: 100 * 1024 * 1024,
        max_task_db_size: 10 * 1024 * 1024,
    };
    let meili = Meilisearch::new(options)
        .expect("failed to create Meilisearch")
        .with_vector_store(store.clone() as Arc<dyn wilysearch::core::VectorStore>);
    (meili, store, temp_dir)
}

#[test]
fn test_add_documents_with_vectors_syncs_to_store() {
    let (meili, store, _tmp) = setup();
    let index = meili.create_index("movies", Some("id")).unwrap();

    let docs = vec![
        json!({
            "id": 1,
            "title": "Interstellar",
            "_vectors": {
                "default": [1.0, 0.0, 0.0, 0.0]
            }
        }),
        json!({
            "id": 2,
            "title": "Inception",
            "_vectors": {
                "default": [0.0, 1.0, 0.0, 0.0]
            }
        }),
    ];

    index.add_documents(docs, None).unwrap();

    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.len(), 2, "store should have 2 documents");

    // Internal IDs are assigned by milli; verify both have vectors
    let all_vecs: Vec<_> = snapshot.values().collect();
    assert!(all_vecs.iter().all(|v| v.len() == 1 && v[0].len() == 4));
}

#[test]
fn test_add_documents_without_vectors_leaves_store_empty() {
    let (meili, store, _tmp) = setup();
    let index = meili.create_index("movies", Some("id")).unwrap();

    let docs = vec![
        json!({"id": 1, "title": "Interstellar"}),
        json!({"id": 2, "title": "Inception"}),
    ];

    index.add_documents(docs, None).unwrap();
    assert!(store.is_empty().unwrap(), "store should be empty when no _vectors");
}

#[test]
fn test_update_documents_with_vectors_syncs_to_store() {
    let (meili, store, _tmp) = setup();
    let index = meili.create_index("movies", Some("id")).unwrap();

    // First add without vectors
    let docs = vec![
        json!({"id": 1, "title": "Interstellar"}),
        json!({"id": 2, "title": "Inception"}),
    ];
    index.add_documents(docs, None).unwrap();
    assert!(store.is_empty().unwrap());

    // Update with vectors
    let updates = vec![json!({
        "id": 1,
        "title": "Interstellar (Updated)",
        "_vectors": {
            "default": [0.5, 0.5, 0.0, 0.0]
        }
    })];

    index.update_documents(updates, None).unwrap();

    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.len(), 1, "only doc 1 should have vectors");
}

#[test]
fn test_delete_documents_removes_from_store() {
    let (meili, store, _tmp) = setup();
    let index = meili.create_index("movies", Some("id")).unwrap();

    let docs = vec![
        json!({
            "id": 1,
            "title": "Interstellar",
            "_vectors": {"default": [1.0, 0.0, 0.0, 0.0]}
        }),
        json!({
            "id": 2,
            "title": "Inception",
            "_vectors": {"default": [0.0, 1.0, 0.0, 0.0]}
        }),
        json!({
            "id": 3,
            "title": "Tenet",
            "_vectors": {"default": [0.0, 0.0, 1.0, 0.0]}
        }),
    ];

    index.add_documents(docs, None).unwrap();
    assert_eq!(store.len().unwrap(), 3);

    // Delete doc 2
    let deleted = index.delete_documents(vec!["2".to_string()]).unwrap();
    assert_eq!(deleted, 1);
    assert_eq!(store.len().unwrap(), 2, "store should have 2 docs after deleting 1");
}

#[test]
fn test_delete_by_filter_removes_from_store() {
    let (meili, store, _tmp) = setup();
    let index = meili.create_index("movies", Some("id")).unwrap();

    // Add docs with filterable year
    let docs = vec![
        json!({
            "id": 1,
            "title": "Interstellar",
            "year": 2014,
            "_vectors": {"default": [1.0, 0.0, 0.0, 0.0]}
        }),
        json!({
            "id": 2,
            "title": "The Dark Knight",
            "year": 2008,
            "_vectors": {"default": [0.0, 1.0, 0.0, 0.0]}
        }),
        json!({
            "id": 3,
            "title": "Inception",
            "year": 2010,
            "_vectors": {"default": [0.0, 0.0, 1.0, 0.0]}
        }),
    ];

    index.add_documents(docs, None).unwrap();
    assert_eq!(store.len().unwrap(), 3);

    // Configure filterable attributes
    let settings = wilysearch::core::Settings {
        filterable_attributes: Some(vec!["year".to_string()]),
        ..Default::default()
    };
    index.update_settings(&settings).unwrap();

    // Delete all movies before 2010
    let deleted = index.delete_by_filter("year < 2010").unwrap();
    assert_eq!(deleted, 1); // The Dark Knight (2008)
    assert_eq!(store.len().unwrap(), 2, "store should have 2 docs after filter delete");
}

#[test]
fn test_clear_empties_store() {
    let (meili, store, _tmp) = setup();
    let index = meili.create_index("movies", Some("id")).unwrap();

    let docs = vec![
        json!({
            "id": 1,
            "title": "Interstellar",
            "_vectors": {"default": [1.0, 0.0, 0.0, 0.0]}
        }),
        json!({
            "id": 2,
            "title": "Inception",
            "_vectors": {"default": [0.0, 1.0, 0.0, 0.0]}
        }),
    ];

    index.add_documents(docs, None).unwrap();
    assert_eq!(store.len().unwrap(), 2);

    let cleared = index.clear().unwrap();
    assert_eq!(cleared, 2);
    assert!(store.is_empty().unwrap(), "store should be empty after clear");
}

#[test]
fn test_multi_vector_format() {
    let (meili, store, _tmp) = setup();
    let index = meili.create_index("movies", Some("id")).unwrap();

    let docs = vec![json!({
        "id": 1,
        "title": "Interstellar",
        "_vectors": {
            "default": [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
        }
    })];

    index.add_documents(docs, None).unwrap();

    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.len(), 1);
    // The document should have 2 vectors
    let vecs = snapshot.values().next().unwrap();
    assert_eq!(vecs.len(), 2, "should have 2 vectors for multi-vector format");
}

#[test]
fn test_structured_vectors_format() {
    let (meili, store, _tmp) = setup();
    let index = meili.create_index("movies", Some("id")).unwrap();

    let docs = vec![json!({
        "id": 1,
        "title": "Interstellar",
        "_vectors": {
            "default": {
                "embeddings": [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                "regenerate": false
            }
        }
    })];

    index.add_documents(docs, None).unwrap();

    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.len(), 1);
    let vecs = snapshot.values().next().unwrap();
    assert_eq!(vecs.len(), 2, "structured format should produce 2 vectors");
}

#[test]
fn test_multiple_embedders() {
    let (meili, store, _tmp) = setup();
    let index = meili.create_index("movies", Some("id")).unwrap();

    let docs = vec![json!({
        "id": 1,
        "title": "Interstellar",
        "_vectors": {
            "openai": [1.0, 0.0, 0.0],
            "cohere": [0.0, 1.0, 0.0]
        }
    })];

    index.add_documents(docs, None).unwrap();

    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.len(), 1);
    // Should have 2 vectors (one from each embedder)
    let vecs = snapshot.values().next().unwrap();
    assert_eq!(vecs.len(), 2, "should collect vectors from all embedders");
}

#[test]
fn test_string_primary_key_with_vectors() {
    let (meili, store, _tmp) = setup();
    let index = meili.create_index("docs", Some("slug")).unwrap();

    let docs = vec![
        json!({
            "slug": "hello-world",
            "title": "Hello World",
            "_vectors": {"default": [1.0, 0.0]}
        }),
        json!({
            "slug": "goodbye-world",
            "title": "Goodbye World",
            "_vectors": {"default": [0.0, 1.0]}
        }),
    ];

    index.add_documents(docs, None).unwrap();

    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.len(), 2, "string PKs should work for vector sync");
}
