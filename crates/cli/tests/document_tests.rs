use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

fn wily() -> Command {
    cargo_bin_cmd!("wily")
}

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

/// Parse stdout bytes as JSON.
fn parse_json(output: &[u8]) -> Value {
    serde_json::from_slice(output).expect("stdout should be valid JSON")
}

// ─── Document add ────────────────────────────────────────────────────────────

#[test]
fn test_doc_add() {
    let tmp = TempDir::new().unwrap();
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
        r#"[{"id":1,"title":"The Matrix","year":1999}]"#,
    )
    .unwrap();

    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "add", "movies"])
        .arg(&doc_file)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json = parse_json(&output);
    assert!(json["taskUid"].is_number(), "response should contain taskUid");
    assert_eq!(json["status"], "succeeded");
    assert_eq!(json["type"], "documentAdditionOrUpdate");
}

#[test]
fn test_doc_add_with_primary_key() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path();

    // Create index WITHOUT a primary key
    wily()
        .arg("--db")
        .arg(db)
        .args(["index", "create", "movies"])
        .assert()
        .success();

    let doc_file = tmp.path().join("movies.json");
    std::fs::write(
        &doc_file,
        r#"[{"id":1,"title":"The Matrix"},{"id":2,"title":"Inception"}]"#,
    )
    .unwrap();

    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "add", "movies", "--primary-key", "id"])
        .arg(&doc_file)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json = parse_json(&output);
    assert_eq!(json["status"], "succeeded");
}

#[test]
fn test_doc_add_missing_file() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path();

    wily()
        .arg("--db")
        .arg(db)
        .args(["index", "create", "movies", "--primary-key", "id"])
        .assert()
        .success();

    wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "add", "movies", "/nonexistent/file.json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error").or(predicate::str::contains("Error")));
}

#[test]
fn test_doc_add_invalid_json() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path();

    wily()
        .arg("--db")
        .arg(db)
        .args(["index", "create", "movies", "--primary-key", "id"])
        .assert()
        .success();

    let bad_file = tmp.path().join("bad.json");
    std::fs::write(&bad_file, "this is not json {{{").unwrap();

    wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "add", "movies"])
        .arg(&bad_file)
        .assert()
        .failure()
        .stderr(predicate::str::contains("error").or(predicate::str::contains("Error")));
}

// ─── Document get ────────────────────────────────────────────────────────────

#[test]
fn test_doc_get() {
    let tmp = TempDir::new().unwrap();
    let db = setup_movies(&tmp);

    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "get", "movies", "1"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json = parse_json(&output);
    assert_eq!(json["id"], 1);
    assert_eq!(json["title"], "The Matrix");
}

#[test]
fn test_doc_get_with_fields() {
    let tmp = TempDir::new().unwrap();
    let db = setup_movies(&tmp);

    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "get", "movies", "2", "--fields", "title,year"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json = parse_json(&output);
    assert!(json["title"].is_string(), "should contain title");
    assert!(json["year"].is_number(), "should contain year");
    assert!(json.get("genres").is_none() || json["genres"].is_null(), "should NOT contain genres");
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
        .failure();
}

// ─── Document list ───────────────────────────────────────────────────────────

#[test]
fn test_doc_list() {
    let tmp = TempDir::new().unwrap();
    let db = setup_movies(&tmp);

    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "list", "movies"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json = parse_json(&output);
    let results = json["results"].as_array().expect("results should be an array");
    assert_eq!(results.len(), 3);
    assert_eq!(json["total"], 3);
}

#[test]
fn test_doc_list_with_limit() {
    let tmp = TempDir::new().unwrap();
    let db = setup_movies(&tmp);

    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "list", "movies", "--limit", "2"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json = parse_json(&output);
    let results = json["results"].as_array().expect("results should be an array");
    assert_eq!(results.len(), 2);
    assert_eq!(json["total"], 3);
}

#[test]
fn test_doc_list_with_offset() {
    let tmp = TempDir::new().unwrap();
    let db = setup_movies(&tmp);

    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "list", "movies", "--offset", "1", "--limit", "2"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json = parse_json(&output);
    let results = json["results"].as_array().expect("results should be an array");
    assert_eq!(results.len(), 2);
}

#[test]
fn test_doc_list_with_fields() {
    let tmp = TempDir::new().unwrap();
    let db = setup_movies(&tmp);

    // The engine only applies field projection when a filter/ids/sort is also
    // present (it takes the `get_documents_with_options` code path). Make `year`
    // filterable and apply a pass-through filter so the fields flag is honored.
    let settings_file = tmp.path().join("settings.json");
    std::fs::write(&settings_file, r#"{"filterableAttributes":["year"]}"#).unwrap();

    wily()
        .arg("--db")
        .arg(db)
        .args(["settings", "update", "movies"])
        .arg(&settings_file)
        .assert()
        .success();

    let output = wily()
        .arg("--db")
        .arg(db)
        .args([
            "doc", "list", "movies", "--fields", "id,title", "--filter", "year >= 0",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json = parse_json(&output);
    let results = json["results"].as_array().expect("results should be an array");
    assert!(!results.is_empty());

    for doc in results {
        assert!(doc["id"].is_number(), "each doc should have id");
        assert!(doc["title"].is_string(), "each doc should have title");
        assert!(
            doc.get("genres").is_none() || doc["genres"].is_null(),
            "docs should NOT contain genres"
        );
    }
}

// ─── Document delete ─────────────────────────────────────────────────────────

#[test]
fn test_doc_delete() {
    let tmp = TempDir::new().unwrap();
    let db = setup_movies(&tmp);

    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "delete", "movies", "1"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json = parse_json(&output);
    assert_eq!(json["status"], "succeeded");

    // Verify doc 1 is gone
    wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "get", "movies", "1"])
        .assert()
        .failure();
}

#[test]
fn test_doc_delete_all() {
    let tmp = TempDir::new().unwrap();
    let db = setup_movies(&tmp);

    wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "delete-all", "movies"])
        .assert()
        .success();

    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "list", "movies"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json = parse_json(&output);
    assert_eq!(json["total"], 0);
}

// ─── Document delete-batch ───────────────────────────────────────────────────

#[test]
fn test_doc_delete_batch() {
    let tmp = TempDir::new().unwrap();
    let db = setup_movies(&tmp);

    wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "delete-batch", "movies", "--ids", "1,2"])
        .assert()
        .success();

    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "list", "movies"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json = parse_json(&output);
    assert_eq!(json["total"], 1);

    let results = json["results"].as_array().expect("results should be an array");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["id"], 3, "only Pulp Fiction (id 3) should remain");
}

// ─── Document delete-filter ──────────────────────────────────────────────────

#[test]
fn test_doc_delete_filter() {
    let tmp = TempDir::new().unwrap();
    let db = setup_movies(&tmp);

    // Make `year` filterable via settings update
    let settings_file = tmp.path().join("settings.json");
    std::fs::write(
        &settings_file,
        r#"{"filterableAttributes":["year"]}"#,
    )
    .unwrap();

    wily()
        .arg("--db")
        .arg(db)
        .args(["settings", "update", "movies"])
        .arg(&settings_file)
        .assert()
        .success();

    // Delete docs where year < 2000 (Matrix 1999 and Pulp Fiction 1994)
    wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "delete-filter", "movies", "--filter", "year < 2000"])
        .assert()
        .success();

    let output = wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "list", "movies"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json = parse_json(&output);
    let results = json["results"].as_array().expect("results should be an array");
    assert_eq!(results.len(), 1, "only Inception should remain");
    assert_eq!(results[0]["title"], "Inception");
    assert_eq!(results[0]["year"], 2010);
}
