mod common;

use std::collections::HashMap;

use common::TestContext;
use wilysearch::core::MeilisearchOptions;
use wilysearch::engine::Engine;
use wilysearch::traits::*;
use wilysearch::types::*;

#[test]
fn test_create_index() {
    let ctx = TestContext::new();
    let result = ctx.engine.create_index(&CreateIndexRequest {
        uid: "movies".to_string(),
        primary_key: None,
    });
    assert!(result.is_ok());
    let task = result.unwrap();
    assert_eq!(task.index_uid, Some("movies".to_string()));
}

#[test]
fn test_create_index_with_primary_key() {
    let ctx = TestContext::new();
    ctx.engine
        .create_index(&CreateIndexRequest {
            uid: "movies".to_string(),
            primary_key: Some("movie_id".to_string()),
        })
        .expect("failed to create index");

    let index = ctx.engine.get_index("movies").expect("failed to get index");
    assert_eq!(index.primary_key, Some("movie_id".to_string()));
}

#[test]
fn test_get_index() {
    let ctx = TestContext::new();
    ctx.engine
        .create_index(&CreateIndexRequest {
            uid: "movies".to_string(),
            primary_key: None,
        })
        .expect("failed to create index");

    let index = ctx.engine.get_index("movies");
    assert!(index.is_ok());
    assert_eq!(index.unwrap().uid, "movies");
}

#[test]
fn test_index_not_found() {
    let ctx = TestContext::new();
    let result = ctx.engine.get_index("nonexistent");
    assert!(result.is_err());
}

#[test]
fn test_delete_index() {
    let ctx = TestContext::new();
    ctx.engine
        .create_index(&CreateIndexRequest {
            uid: "movies".to_string(),
            primary_key: None,
        })
        .expect("failed to create index");

    // Verify it exists
    assert!(ctx.engine.get_index("movies").is_ok());

    ctx.engine
        .delete_index("movies")
        .expect("failed to delete index");

    // Verify it's gone
    assert!(ctx.engine.get_index("movies").is_err());
}

#[test]
fn test_list_indexes() {
    let ctx = TestContext::new();
    for name in &["alpha", "beta", "gamma"] {
        ctx.engine
            .create_index(&CreateIndexRequest {
                uid: name.to_string(),
                primary_key: None,
            })
            .expect("failed to create index");
    }

    let list = ctx
        .engine
        .list_indexes(&PaginationQuery {
            offset: None,
            limit: Some(20),
        })
        .expect("failed to list indexes");

    assert_eq!(list.total, 3);
    let uids: Vec<&str> = list.results.iter().map(|i| i.uid.as_str()).collect();
    assert!(uids.contains(&"alpha"));
    assert!(uids.contains(&"beta"));
    assert!(uids.contains(&"gamma"));
}

#[test]
fn test_list_indexes_with_pagination() {
    let ctx = TestContext::new();
    for name in &["aaa", "bbb", "ccc", "ddd", "eee"] {
        ctx.engine
            .create_index(&CreateIndexRequest {
                uid: name.to_string(),
                primary_key: None,
            })
            .expect("failed to create index");
    }

    let page1 = ctx
        .engine
        .list_indexes(&PaginationQuery {
            offset: Some(0),
            limit: Some(2),
        })
        .expect("failed to list page 1");
    assert_eq!(page1.total, 5);
    assert_eq!(page1.results.len(), 2);

    let page2 = ctx
        .engine
        .list_indexes(&PaginationQuery {
            offset: Some(2),
            limit: Some(2),
        })
        .expect("failed to list page 2");
    assert_eq!(page2.results.len(), 2);

    let page3 = ctx
        .engine
        .list_indexes(&PaginationQuery {
            offset: Some(4),
            limit: Some(2),
        })
        .expect("failed to list page 3");
    assert_eq!(page3.results.len(), 1);
}

#[test]
fn test_index_stats() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");

    let expected_count = common::sample_movies().len() as u64;
    let stats = ctx
        .engine
        .index_stats("movies")
        .expect("failed to get stats");
    assert_eq!(stats.number_of_documents, expected_count);
    assert!(stats.field_distribution.contains_key("title"));
}

#[test]
fn test_health() {
    let ctx = TestContext::new();
    let health = ctx.engine.health().expect("health check failed");
    assert_eq!(health.status, "available");
}

#[test]
fn test_version() {
    let ctx = TestContext::new();
    let version = ctx.engine.version().expect("version check failed");
    assert!(!version.pkg_version.is_empty());
}

#[test]
fn test_index_has_created_at() {
    let ctx = TestContext::new();
    ctx.engine
        .create_index(&CreateIndexRequest {
            uid: "movies".to_string(),
            primary_key: None,
        })
        .expect("failed to create index");

    let index = ctx.engine.get_index("movies").expect("failed to get index");
    assert!(!index.created_at.is_empty(), "created_at should be set");
    assert!(!index.updated_at.is_empty(), "updated_at should be set");
    // Verify ISO-8601 format (starts with a year)
    assert!(index.created_at.starts_with("20"), "created_at should be ISO-8601");
    assert!(index.updated_at.starts_with("20"), "updated_at should be ISO-8601");
}

#[test]
fn test_updated_at_changes_on_document_add() {
    let ctx = TestContext::new();
    ctx.engine
        .create_index(&CreateIndexRequest {
            uid: "movies".to_string(),
            primary_key: Some("id".to_string()),
        })
        .expect("failed to create index");

    let before = ctx.engine.get_index("movies").unwrap();
    let created_at_before = before.created_at.clone();
    let updated_at_before = before.updated_at.clone();

    // Sleep to ensure timestamp advances. RFC3339 format includes sub-second
    // precision, but 10ms can be unreliable on loaded systems due to OS timer
    // granularity. 100ms provides a comfortable margin.
    std::thread::sleep(std::time::Duration::from_millis(100));

    ctx.engine
        .add_or_replace_documents(
            "movies",
            &common::sample_movies(),
            &AddDocumentsQuery::default(),
        )
        .expect("failed to add documents");

    let after = ctx.engine.get_index("movies").unwrap();
    assert_eq!(after.created_at, created_at_before, "created_at should not change");
    assert!(after.updated_at > updated_at_before, "updated_at should advance");
}

#[test]
fn test_global_stats_last_update() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");

    let stats = ctx.engine.global_stats().expect("failed to get global stats");
    assert!(stats.last_update.is_some(), "last_update should be Some after mutations");
    let ts = stats.last_update.unwrap();
    assert!(ts.starts_with("20"), "last_update should be ISO-8601");
}

#[test]
fn test_list_indexes_has_timestamps() {
    let ctx = TestContext::new();
    ctx.engine
        .create_index(&CreateIndexRequest {
            uid: "movies".to_string(),
            primary_key: None,
        })
        .expect("failed to create index");

    let list = ctx
        .engine
        .list_indexes(&PaginationQuery {
            offset: None,
            limit: Some(20),
        })
        .expect("failed to list indexes");

    assert_eq!(list.results.len(), 1);
    let idx = &list.results[0];
    assert!(!idx.created_at.is_empty(), "created_at should be set in list");
    assert!(!idx.updated_at.is_empty(), "updated_at should be set in list");
}

#[test]
fn test_timestamps_persist_across_restart() {
    let temp_dir = tempfile::TempDir::new().expect("failed to create temp dir");
    let db_path = temp_dir.path().to_path_buf();

    let created_at;
    {
        let engine = Engine::new(MeilisearchOptions {
            db_path: db_path.clone(),
            max_index_size: 100 * 1024 * 1024,
            max_task_db_size: 10 * 1024 * 1024,
        })
        .expect("failed to create engine");

        engine
            .create_index(&CreateIndexRequest {
                uid: "movies".to_string(),
                primary_key: None,
            })
            .expect("failed to create index");

        let index = engine.get_index("movies").unwrap();
        created_at = index.created_at.clone();
        assert!(!created_at.is_empty());
    }

    // Reopen with a fresh Engine pointing at the same db_path
    let engine2 = Engine::new(MeilisearchOptions {
        db_path,
        max_index_size: 100 * 1024 * 1024,
        max_task_db_size: 10 * 1024 * 1024,
    })
    .expect("failed to reopen engine");

    let index = engine2.get_index("movies").unwrap();
    assert_eq!(index.created_at, created_at, "created_at should survive restart");
    assert!(!index.updated_at.is_empty(), "updated_at should survive restart");
}

#[test]
fn test_export_all_indexes() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");
    common::create_test_index(&ctx, "books");

    let export_dir = tempfile::TempDir::new().expect("failed to create export dir");
    let result = ctx.engine.export(&ExportRequest {
        url: export_dir.path().to_string_lossy().to_string(),
        api_key: None,
        indexes: None,
    });
    assert!(result.is_ok(), "export should succeed: {result:?}");

    // Both index dirs should exist with documents.json and settings.json
    for uid in &["movies", "books"] {
        let index_dir = export_dir.path().join(uid);
        assert!(index_dir.exists(), "{uid} dir should exist");
        assert!(index_dir.join("documents.json").exists(), "{uid}/documents.json should exist");
        assert!(index_dir.join("settings.json").exists(), "{uid}/settings.json should exist");

        // Verify documents.json is valid JSON array with content
        let docs: Vec<serde_json::Value> = serde_json::from_str(
            &std::fs::read_to_string(index_dir.join("documents.json")).unwrap(),
        )
        .expect("documents.json should be valid JSON");
        assert!(!docs.is_empty(), "{uid} should have documents");
    }
}

#[test]
fn test_export_filtered_indexes() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");
    common::create_test_index(&ctx, "books");
    common::create_test_index(&ctx, "songs");

    let export_dir = tempfile::TempDir::new().expect("failed to create export dir");

    let mut indexes = HashMap::new();
    indexes.insert("movies".to_string(), ExportIndexConfig { override_settings: None });
    indexes.insert("songs".to_string(), ExportIndexConfig { override_settings: None });

    let result = ctx.engine.export(&ExportRequest {
        url: export_dir.path().to_string_lossy().to_string(),
        api_key: None,
        indexes: Some(indexes),
    });
    assert!(result.is_ok(), "export should succeed: {result:?}");

    // Only movies and songs should be exported
    assert!(export_dir.path().join("movies").exists(), "movies should be exported");
    assert!(export_dir.path().join("songs").exists(), "songs should be exported");
    assert!(!export_dir.path().join("books").exists(), "books should NOT be exported");
}

#[test]
fn test_export_without_settings() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");

    let export_dir = tempfile::TempDir::new().expect("failed to create export dir");

    let mut indexes = HashMap::new();
    indexes.insert(
        "movies".to_string(),
        ExportIndexConfig { override_settings: Some(false) },
    );

    let result = ctx.engine.export(&ExportRequest {
        url: export_dir.path().to_string_lossy().to_string(),
        api_key: None,
        indexes: Some(indexes),
    });
    assert!(result.is_ok(), "export should succeed: {result:?}");

    let index_dir = export_dir.path().join("movies");
    assert!(index_dir.join("documents.json").exists(), "documents.json should exist");
    assert!(!index_dir.join("settings.json").exists(), "settings.json should NOT exist when override_settings=false");
}
