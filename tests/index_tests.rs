mod common;

use common::TestContext;
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

    let stats = ctx
        .engine
        .index_stats("movies")
        .expect("failed to get stats");
    assert_eq!(stats.number_of_documents, 10);
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
