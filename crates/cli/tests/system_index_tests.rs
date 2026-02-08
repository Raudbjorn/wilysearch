use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

fn wily() -> Command {
    cargo_bin_cmd!("wily")
}

// ─── System commands ─────────────────────────────────────────────────────────

#[test]
fn test_health() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("db.ms");

    let output = wily()
        .arg("--db")
        .arg(&db)
        .arg("health")
        .output()
        .unwrap();

    assert!(output.status.success(), "health should exit 0");
    let json: Value = serde_json::from_slice(&output.stdout)
        .expect("health output should be valid JSON");
    assert_eq!(json["status"], "available", "status should be 'available'");
}

#[test]
fn test_version() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("db.ms");

    let output = wily()
        .arg("--db")
        .arg(&db)
        .arg("version")
        .output()
        .unwrap();

    assert!(output.status.success(), "version should exit 0");
    let json: Value = serde_json::from_slice(&output.stdout)
        .expect("version output should be valid JSON");
    assert!(
        json.get("pkgVersion").is_some(),
        "version output should contain pkgVersion"
    );
    assert!(
        json.get("commitSha").is_some(),
        "version output should contain commitSha"
    );
    assert!(
        json.get("commitDate").is_some(),
        "version output should contain commitDate"
    );
}

#[test]
fn test_global_stats_empty() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("db.ms");

    let output = wily()
        .arg("--db")
        .arg(&db)
        .arg("stats")
        .output()
        .unwrap();

    assert!(output.status.success(), "global stats should exit 0");
    let json: Value = serde_json::from_slice(&output.stdout)
        .expect("stats output should be valid JSON");
    assert!(
        json.get("databaseSize").is_some(),
        "global stats should contain databaseSize"
    );
    assert_eq!(
        json["indexes"],
        serde_json::json!({}),
        "global stats on empty db should have empty indexes map"
    );
}

#[test]
fn test_index_stats() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("db.ms");

    // Create index with primary key
    wily()
        .arg("--db")
        .arg(&db)
        .args(["index", "create", "movies", "--primary-key", "id"])
        .assert()
        .success();

    // Add documents
    let doc_file = tmp.path().join("docs.json");
    std::fs::write(
        &doc_file,
        r#"[{"id":1,"title":"The Matrix"},{"id":2,"title":"Inception"}]"#,
    )
    .unwrap();
    wily()
        .arg("--db")
        .arg(&db)
        .args(["doc", "add", "movies"])
        .arg(&doc_file)
        .assert()
        .success();

    // Get index stats
    let output = wily()
        .arg("--db")
        .arg(&db)
        .args(["stats", "movies"])
        .output()
        .unwrap();

    assert!(output.status.success(), "index stats should exit 0");
    let json: Value = serde_json::from_slice(&output.stdout)
        .expect("index stats output should be valid JSON");
    assert!(
        json.get("numberOfDocuments").is_some(),
        "index stats should contain numberOfDocuments"
    );
    assert_eq!(
        json["numberOfDocuments"], 2,
        "numberOfDocuments should be 2 after adding 2 docs"
    );
}

#[test]
fn test_index_stats_not_found() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("db.ms");

    wily()
        .arg("--db")
        .arg(&db)
        .args(["stats", "nonexistent"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("error"));
}

#[test]
fn test_dump() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("db.ms");

    let output = wily()
        .arg("--db")
        .arg(&db)
        .arg("dump")
        .output()
        .unwrap();

    assert!(output.status.success(), "dump should exit 0");
    let json: Value = serde_json::from_slice(&output.stdout)
        .expect("dump output should be valid JSON");
    assert_eq!(
        json["type"], "dumpCreation",
        "dump task type should be 'dumpCreation'"
    );
    assert_eq!(
        json["status"], "succeeded",
        "dump task status should be 'succeeded'"
    );
}

#[test]
fn test_snapshot() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("db.ms");

    let output = wily()
        .arg("--db")
        .arg(&db)
        .arg("snapshot")
        .output()
        .unwrap();

    assert!(output.status.success(), "snapshot should exit 0");
    let json: Value = serde_json::from_slice(&output.stdout)
        .expect("snapshot output should be valid JSON");
    assert_eq!(
        json["type"], "snapshotCreation",
        "snapshot task type should be 'snapshotCreation'"
    );
    assert_eq!(
        json["status"], "succeeded",
        "snapshot task status should be 'succeeded'"
    );
}

#[test]
fn test_no_subcommand() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("db.ms");

    wily()
        .arg("--db")
        .arg(&db)
        .assert()
        .failure()
        .code(2);
}

// ─── Index commands ──────────────────────────────────────────────────────────

#[test]
fn test_index_create() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("db.ms");

    let output = wily()
        .arg("--db")
        .arg(&db)
        .args(["index", "create", "movies", "--primary-key", "id"])
        .output()
        .unwrap();

    assert!(output.status.success(), "index create should exit 0");
    let json: Value = serde_json::from_slice(&output.stdout)
        .expect("index create output should be valid JSON");
    assert!(
        json.get("taskUid").is_some(),
        "index create should return taskUid"
    );
    assert_eq!(
        json["status"], "succeeded",
        "index create status should be 'succeeded'"
    );
    assert_eq!(
        json["type"], "indexCreation",
        "index create type should be 'indexCreation'"
    );
    assert_eq!(
        json["indexUid"], "movies",
        "indexUid should be 'movies'"
    );
}

#[test]
fn test_index_create_without_pk() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("db.ms");

    wily()
        .arg("--db")
        .arg(&db)
        .args(["index", "create", "movies"])
        .assert()
        .success();
}

#[test]
fn test_index_list_empty() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("db.ms");

    let output = wily()
        .arg("--db")
        .arg(&db)
        .args(["index", "list"])
        .output()
        .unwrap();

    assert!(output.status.success(), "index list should exit 0");
    let json: Value = serde_json::from_slice(&output.stdout)
        .expect("index list output should be valid JSON");
    assert_eq!(
        json["results"],
        serde_json::json!([]),
        "results should be an empty array"
    );
    assert_eq!(json["total"], 0, "total should be 0 for empty db");
}

#[test]
fn test_index_list_after_create() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("db.ms");

    // Create an index
    wily()
        .arg("--db")
        .arg(&db)
        .args(["index", "create", "movies", "--primary-key", "id"])
        .assert()
        .success();

    // List indexes
    let output = wily()
        .arg("--db")
        .arg(&db)
        .args(["index", "list"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout)
        .expect("index list output should be valid JSON");
    assert_eq!(json["total"], 1, "total should be 1 after creating one index");

    let results = json["results"].as_array().expect("results should be an array");
    assert_eq!(results.len(), 1, "results should contain 1 index");
    assert_eq!(
        results[0]["uid"], "movies",
        "the listed index uid should be 'movies'"
    );
}

#[test]
fn test_index_list_pagination() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("db.ms");

    // Create 3 indexes
    for name in &["alpha", "beta", "gamma"] {
        wily()
            .arg("--db")
            .arg(&db)
            .args(["index", "create", name])
            .assert()
            .success();
    }

    // List with limit 2
    let output = wily()
        .arg("--db")
        .arg(&db)
        .args(["index", "list", "--limit", "2"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["total"], 3, "total should be 3 regardless of limit");
    let results = json["results"].as_array().expect("results should be an array");
    assert_eq!(results.len(), 2, "results should contain 2 items with limit 2");

    // List with offset 2, limit 2 — should get the remaining 1
    let output = wily()
        .arg("--db")
        .arg(&db)
        .args(["index", "list", "--offset", "2", "--limit", "2"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["total"], 3, "total should still be 3");
    let results = json["results"].as_array().expect("results should be an array");
    assert_eq!(
        results.len(),
        1,
        "results should contain 1 item when offset past most entries"
    );
}

#[test]
fn test_index_get() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("db.ms");

    // Create an index
    wily()
        .arg("--db")
        .arg(&db)
        .args(["index", "create", "movies", "--primary-key", "id"])
        .assert()
        .success();

    // Get the index
    let output = wily()
        .arg("--db")
        .arg(&db)
        .args(["index", "get", "movies"])
        .output()
        .unwrap();

    assert!(output.status.success(), "index get should exit 0");
    let json: Value = serde_json::from_slice(&output.stdout)
        .expect("index get output should be valid JSON");
    assert_eq!(json["uid"], "movies", "uid should be 'movies'");
    assert_eq!(json["primaryKey"], "id", "primaryKey should be 'id'");
    assert!(
        json.get("createdAt").is_some(),
        "index get should contain createdAt"
    );
    assert!(
        json.get("updatedAt").is_some(),
        "index get should contain updatedAt"
    );
}

#[test]
fn test_index_get_not_found() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("db.ms");

    wily()
        .arg("--db")
        .arg(&db)
        .args(["index", "get", "nonexistent"])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn test_index_update() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("db.ms");

    // Create index without primary key
    wily()
        .arg("--db")
        .arg(&db)
        .args(["index", "create", "movies"])
        .assert()
        .success();

    // Update with primary key
    let output = wily()
        .arg("--db")
        .arg(&db)
        .args(["index", "update", "movies", "--primary-key", "id"])
        .output()
        .unwrap();

    assert!(output.status.success(), "index update should exit 0");
    let json: Value = serde_json::from_slice(&output.stdout)
        .expect("index update output should be valid JSON");
    assert_eq!(
        json["status"], "succeeded",
        "index update status should be 'succeeded'"
    );

    // Verify the primary key was set
    let output = wily()
        .arg("--db")
        .arg(&db)
        .args(["index", "get", "movies"])
        .output()
        .unwrap();

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["primaryKey"], "id",
        "primaryKey should be 'id' after update"
    );
}

#[test]
fn test_index_delete() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("db.ms");

    // Create index
    wily()
        .arg("--db")
        .arg(&db)
        .args(["index", "create", "movies"])
        .assert()
        .success();

    // Delete it
    let output = wily()
        .arg("--db")
        .arg(&db)
        .args(["index", "delete", "movies"])
        .output()
        .unwrap();

    assert!(output.status.success(), "index delete should exit 0");
    let json: Value = serde_json::from_slice(&output.stdout)
        .expect("index delete output should be valid JSON");
    assert_eq!(
        json["status"], "succeeded",
        "index delete status should be 'succeeded'"
    );

    // Verify it's gone
    wily()
        .arg("--db")
        .arg(&db)
        .args(["index", "get", "movies"])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn test_index_swap() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("db.ms");

    // Create two indexes with primary keys
    wily()
        .arg("--db")
        .arg(&db)
        .args(["index", "create", "movies", "--primary-key", "id"])
        .assert()
        .success();
    wily()
        .arg("--db")
        .arg(&db)
        .args(["index", "create", "books", "--primary-key", "id"])
        .assert()
        .success();

    // Add different docs to each
    let movies_file = tmp.path().join("movies.json");
    std::fs::write(
        &movies_file,
        r#"[{"id":1,"title":"The Matrix"}]"#,
    )
    .unwrap();
    wily()
        .arg("--db")
        .arg(&db)
        .args(["doc", "add", "movies"])
        .arg(&movies_file)
        .assert()
        .success();

    let books_file = tmp.path().join("books.json");
    std::fs::write(
        &books_file,
        r#"[{"id":1,"title":"Dune"},{"id":2,"title":"Neuromancer"}]"#,
    )
    .unwrap();
    wily()
        .arg("--db")
        .arg(&db)
        .args(["doc", "add", "books"])
        .arg(&books_file)
        .assert()
        .success();

    // Verify doc counts before swap
    let output = wily()
        .arg("--db")
        .arg(&db)
        .args(["stats", "movies"])
        .output()
        .unwrap();
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["numberOfDocuments"], 1,
        "movies should have 1 document before swap"
    );

    let output = wily()
        .arg("--db")
        .arg(&db)
        .args(["stats", "books"])
        .output()
        .unwrap();
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["numberOfDocuments"], 2,
        "books should have 2 documents before swap"
    );

    // Swap the indexes
    let output = wily()
        .arg("--db")
        .arg(&db)
        .args(["index", "swap", "movies", "books"])
        .output()
        .unwrap();

    assert!(output.status.success(), "index swap should exit 0");
    let json: Value = serde_json::from_slice(&output.stdout)
        .expect("index swap output should be valid JSON");
    assert_eq!(
        json["status"], "succeeded",
        "index swap status should be 'succeeded'"
    );

    // Verify docs moved: movies should now have 2, books should have 1
    let output = wily()
        .arg("--db")
        .arg(&db)
        .args(["stats", "movies"])
        .output()
        .unwrap();
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["numberOfDocuments"], 2,
        "movies should have 2 documents after swap (was books' data)"
    );

    let output = wily()
        .arg("--db")
        .arg(&db)
        .args(["stats", "books"])
        .output()
        .unwrap();
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["numberOfDocuments"], 1,
        "books should have 1 document after swap (was movies' data)"
    );
}
