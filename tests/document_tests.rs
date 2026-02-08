mod common;

use common::TestContext;
use serde_json::json;
use wilysearch::traits::*;
use wilysearch::types::*;

#[test]
fn test_add_documents() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");

    let resp = ctx
        .engine
        .get_documents("movies", &DocumentsQuery { limit: Some(100), ..Default::default() })
        .expect("failed to get documents");
    assert_eq!(resp.total, 10);
}

#[test]
fn test_get_document() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");

    let doc = ctx
        .engine
        .get_document("movies", "1", &DocumentQuery::default())
        .expect("failed to get document");
    assert_eq!(doc["title"], "The Dark Knight");
}

#[test]
fn test_get_documents_paginated() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");

    let page1 = ctx
        .engine
        .get_documents("movies", &DocumentsQuery {
            offset: Some(0),
            limit: Some(5),
            ..Default::default()
        })
        .expect("failed to get documents");
    assert_eq!(page1.results.len(), 5);
    assert_eq!(page1.total, 10);
    assert_eq!(page1.offset, 0);

    let page2 = ctx
        .engine
        .get_documents("movies", &DocumentsQuery {
            offset: Some(5),
            limit: Some(5),
            ..Default::default()
        })
        .expect("failed to get page 2");
    assert_eq!(page2.results.len(), 5);
    assert_eq!(page2.offset, 5);
}

#[test]
fn test_update_documents_partial() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");

    let updates = vec![json!({ "id": 1, "rating": 9.5 })];
    ctx.engine
        .add_or_update_documents("movies", &updates, &AddDocumentsQuery::default())
        .expect("failed to update documents");

    let doc = ctx
        .engine
        .get_document("movies", "1", &DocumentQuery::default())
        .expect("failed to get document");

    assert_eq!(doc["rating"], 9.5);
    // Title should still be present (not wiped by partial update)
    assert_eq!(doc["title"], "The Dark Knight");
}

#[test]
fn test_delete_document() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");

    ctx.engine
        .delete_document("movies", "1")
        .expect("failed to delete");

    let resp = ctx
        .engine
        .get_documents("movies", &DocumentsQuery { limit: Some(100), ..Default::default() })
        .expect("failed to get documents");
    assert_eq!(resp.total, 9);
}

#[test]
fn test_delete_documents_batch() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");

    let ids: Vec<serde_json::Value> = vec![json!("1"), json!("2"), json!("3")];
    ctx.engine
        .delete_documents_by_batch("movies", &ids)
        .expect("failed to delete batch");

    let resp = ctx
        .engine
        .get_documents("movies", &DocumentsQuery { limit: Some(100), ..Default::default() })
        .expect("failed to get documents");
    assert_eq!(resp.total, 7);
}

#[test]
fn test_delete_by_filter() {
    let ctx = TestContext::new();
    common::create_configured_index(&ctx, "movies");

    ctx.engine
        .delete_documents_by_filter("movies", &DeleteDocumentsByFilterRequest {
            filter: "year = 1994".to_string(),
        })
        .expect("failed to delete by filter");

    let resp = ctx
        .engine
        .get_documents("movies", &DocumentsQuery { limit: Some(100), ..Default::default() })
        .expect("failed to get documents");
    // 3 movies from 1994 should be deleted: Shawshank, Pulp Fiction, Forrest Gump
    assert_eq!(resp.total, 7);
}

#[test]
fn test_delete_all_documents() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");

    ctx.engine
        .delete_all_documents("movies")
        .expect("failed to delete all");

    let resp = ctx
        .engine
        .get_documents("movies", &DocumentsQuery { limit: Some(100), ..Default::default() })
        .expect("failed to get documents");
    assert_eq!(resp.total, 0);
}

#[test]
fn test_document_not_found() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");

    // Attempting to get a non-existent document should return an error
    let result = ctx
        .engine
        .get_document("movies", "99999", &DocumentQuery::default());
    assert!(result.is_err(), "getting a non-existent document should return an error");
}
