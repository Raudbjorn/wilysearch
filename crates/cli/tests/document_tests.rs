mod common;

use common::{parse_json, wily};
use predicates::prelude::*;
use tempfile::TempDir;

/// Create an index and add sample movie documents. Returns the db path.
fn setup_movies(tmp: &TempDir) -> &std::path::Path {
    let db = tmp.path();

    wily()
        .arg("--db")
        .arg(db)
        .args(["index", "create", "movies", "--primary-key", "id"])
        .assert()
        .success();

    let doc_file = tmp.path().join("movies.json");
    std::fs::write(
        &doc_file,
        r#"[
        {"id":1,"title":"The Matrix","year":1999,"genres":["Sci-Fi"]},
        {"id":2,"title":"Inception","year":2010,"genres":["Sci-Fi","Thriller"]},
        {"id":3,"title":"Pulp Fiction","year":1994,"genres":["Crime","Drama"]}
    ]"#,
    )
    .unwrap();

    wily()
        .arg("--db")
        .arg(db)
        .arg("doc")
        .arg("add")
        .arg("movies")
        .arg(&doc_file)
        .assert()
        .success();

    db
}

// ─── Get document ───────────────────────────────────────────────────────────

#[test]
fn test_doc_get() {
    let tmp = TempDir::new().unwrap();
    let db = setup_movies(&tmp);

    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "get", "movies", "1"])
        .output()
        .unwrap();

    assert!(output.status.success(), "doc get should exit 0");
    let json = parse_json(&output.stdout);
    assert_eq!(json["id"], 1, "doc id should be 1");
    assert_eq!(json["title"], "The Matrix");
}

#[test]
fn test_doc_get_not_found() {
    let tmp = TempDir::new().unwrap();
    let db = setup_movies(&tmp);

    wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "get", "movies", "999"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("error").or(predicate::str::contains("Error")));
}

#[test]
fn test_doc_get_with_fields() {
    let tmp = TempDir::new().unwrap();
    let db = setup_movies(&tmp);

    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "get", "movies", "1", "--fields", "title"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json = parse_json(&output.stdout);
    assert_eq!(json["title"], "The Matrix");
    // id may still be returned (Meilisearch behaviour), but at least title is there
}

// ─── List documents ─────────────────────────────────────────────────────────

#[test]
fn test_doc_list() {
    let tmp = TempDir::new().unwrap();
    let db = setup_movies(&tmp);

    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "list", "movies"])
        .output()
        .unwrap();

    assert!(output.status.success(), "doc list should exit 0");
    let json = parse_json(&output.stdout);
    assert_eq!(json["total"], 3, "total documents should be 3");
    let results = json["results"]
        .as_array()
        .expect("results should be an array");
    assert_eq!(results.len(), 3, "results should contain 3 documents");
}

#[test]
fn test_doc_list_with_limit() {
    let tmp = TempDir::new().unwrap();
    let db = setup_movies(&tmp);

    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "list", "movies", "--limit", "1"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json = parse_json(&output.stdout);
    assert_eq!(json["total"], 3, "total should still be 3");
    let results = json["results"]
        .as_array()
        .expect("results should be an array");
    assert_eq!(results.len(), 1, "results should contain 1 document with limit 1");
}

#[test]
fn test_doc_list_with_offset() {
    let tmp = TempDir::new().unwrap();
    let db = setup_movies(&tmp);

    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "list", "movies", "--offset", "2"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json = parse_json(&output.stdout);
    assert_eq!(json["total"], 3, "total should still be 3");
    let results = json["results"]
        .as_array()
        .expect("results should be an array");
    assert_eq!(
        results.len(),
        1,
        "results should contain 1 document with offset 2 of 3 total"
    );
}

#[test]
fn test_doc_list_with_fields() {
    let tmp = TempDir::new().unwrap();
    let db = setup_movies(&tmp);

    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "list", "movies", "--fields", "id,title"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json = parse_json(&output.stdout);
    let results = json["results"]
        .as_array()
        .expect("results should be an array");
    // Each result should contain at least id and title
    for doc in results {
        assert!(doc.get("id").is_some(), "document should have 'id'");
        assert!(doc.get("title").is_some(), "document should have 'title'");
    }
}

// ─── Add documents ──────────────────────────────────────────────────────────

#[test]
fn test_doc_add() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path();

    // Create index
    wily()
        .arg("--db")
        .arg(db)
        .args(["index", "create", "movies", "--primary-key", "id"])
        .assert()
        .success();

    // Write docs to file
    let doc_file = tmp.path().join("docs.json");
    std::fs::write(
        &doc_file,
        r#"[{"id":1,"title":"The Matrix"}]"#,
    )
    .unwrap();

    // Add docs
    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "add", "movies"])
        .arg(&doc_file)
        .output()
        .unwrap();

    assert!(output.status.success(), "doc add should exit 0");
    let json = parse_json(&output.stdout);
    assert_eq!(
        json["status"], "succeeded",
        "doc add status should be 'succeeded'"
    );
    assert_eq!(
        json["type"], "documentAdditionOrUpdate",
        "task type should be 'documentAdditionOrUpdate'"
    );
}

#[test]
fn test_doc_add_nonexistent_file() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path();

    wily()
        .arg("--db")
        .arg(db)
        .args(["index", "create", "movies"])
        .assert()
        .success();

    wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "add", "movies", "/nonexistent/docs.json"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("error").or(predicate::str::contains("Error")));
}

#[test]
fn test_doc_add_invalid_json() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path();

    wily()
        .arg("--db")
        .arg(db)
        .args(["index", "create", "movies"])
        .assert()
        .success();

    let bad_file = tmp.path().join("bad.json");
    std::fs::write(&bad_file, "{ not valid json !!!").unwrap();

    wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "add", "movies"])
        .arg(&bad_file)
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("error").or(predicate::str::contains("Error")));
}

// ─── Delete single document ─────────────────────────────────────────────────

#[test]
fn test_doc_delete() {
    let tmp = TempDir::new().unwrap();
    let db = setup_movies(&tmp);

    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "delete", "movies", "1"])
        .output()
        .unwrap();

    assert!(output.status.success(), "doc delete should exit 0");
    let json = parse_json(&output.stdout);
    assert_eq!(
        json["status"], "succeeded",
        "doc delete status should be 'succeeded'"
    );

    // Verify the document is gone
    wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "get", "movies", "1"])
        .assert()
        .failure()
        .code(1);
}

// ─── Delete batch ───────────────────────────────────────────────────────────

#[test]
fn test_doc_delete_batch() {
    let tmp = TempDir::new().unwrap();
    let db = setup_movies(&tmp);

    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "delete-batch", "movies", "--ids", "1,2"])
        .output()
        .unwrap();

    assert!(output.status.success(), "doc delete-batch should exit 0");
    let json = parse_json(&output.stdout);
    assert_eq!(json["status"], "succeeded");

    // Verify only doc 3 remains
    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "list", "movies"])
        .output()
        .unwrap();

    let json = parse_json(&output.stdout);
    assert_eq!(json["total"], 1, "should have 1 document after deleting 2");
}

// ─── Delete by filter ───────────────────────────────────────────────────────

#[test]
fn test_doc_delete_filter() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path();

    // Create index
    wily()
        .arg("--db")
        .arg(db)
        .args(["index", "create", "movies", "--primary-key", "id"])
        .assert()
        .success();

    // Configure year as filterable
    let settings_file = tmp.path().join("settings.json");
    std::fs::write(
        &settings_file,
        r#"{"filterableAttributes": ["year"]}"#,
    )
    .unwrap();
    wily()
        .arg("--db")
        .arg(db)
        .args(["settings", "update", "movies"])
        .arg(&settings_file)
        .assert()
        .success();

    // Add docs
    let doc_file = tmp.path().join("docs.json");
    std::fs::write(
        &doc_file,
        r#"[
            {"id":1,"title":"The Matrix","year":1999},
            {"id":2,"title":"Inception","year":2010},
            {"id":3,"title":"Pulp Fiction","year":1994}
        ]"#,
    )
    .unwrap();
    wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "add", "movies"])
        .arg(&doc_file)
        .assert()
        .success();

    // Delete docs from year < 2000
    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "delete-filter", "movies", "--filter", "year < 2000"])
        .output()
        .unwrap();

    assert!(output.status.success(), "doc delete-filter should exit 0");
    let json = parse_json(&output.stdout);
    assert_eq!(json["status"], "succeeded");

    // Verify only year >= 2000 remain
    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "list", "movies"])
        .output()
        .unwrap();

    let json = parse_json(&output.stdout);
    assert_eq!(
        json["total"], 1,
        "should have 1 document after filter delete (only Inception)"
    );
}

// ─── Delete all ─────────────────────────────────────────────────────────────

#[test]
fn test_doc_delete_all() {
    let tmp = TempDir::new().unwrap();
    let db = setup_movies(&tmp);

    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "delete-all", "movies"])
        .output()
        .unwrap();

    assert!(output.status.success(), "doc delete-all should exit 0");
    let json = parse_json(&output.stdout);
    assert_eq!(json["status"], "succeeded");

    // Verify all docs are gone
    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "list", "movies"])
        .output()
        .unwrap();

    let json = parse_json(&output.stdout);
    assert_eq!(json["total"], 0, "should have 0 documents after delete-all");
}
