mod common;

use common::{add_sample_docs, create_index, fresh_db, parse_json, wily};
use predicates::prelude::*;
use tempfile::TempDir;

// ─── Settings commands ───────────────────────────────────────────────────────

#[test]
fn test_settings_get() {
    let db = fresh_db();
    create_index(db.path(), "movies");

    let output = wily()
        .arg("--db")
        .arg(db.path())
        .args(["settings", "get", "movies"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let settings = parse_json(&output);

    assert!(
        settings.get("rankingRules").is_some(),
        "settings should contain rankingRules"
    );
    assert!(
        settings.get("searchableAttributes").is_some(),
        "settings should contain searchableAttributes"
    );
    assert!(
        settings.get("filterableAttributes").is_some(),
        "settings should contain filterableAttributes"
    );
    assert!(
        settings.get("sortableAttributes").is_some(),
        "settings should contain sortableAttributes"
    );
    assert!(
        settings.get("typoTolerance").is_some(),
        "settings should contain typoTolerance"
    );
}

#[test]
fn test_settings_update() {
    let db = fresh_db();
    create_index(db.path(), "movies");

    // Write settings JSON to a temp file
    let settings_dir = TempDir::new().unwrap();
    let settings_path = settings_dir.path().join("settings.json");
    let settings_json = serde_json::json!({
        "filterableAttributes": ["year", "genres"],
        "sortableAttributes": ["year"]
    });
    std::fs::write(&settings_path, serde_json::to_string_pretty(&settings_json).unwrap())
        .expect("failed to write settings file");

    // Update settings
    wily()
        .arg("--db")
        .arg(db.path())
        .args(["settings", "update", "movies"])
        .arg(&settings_path)
        .assert()
        .success();

    // Verify settings were applied
    let output = wily()
        .arg("--db")
        .arg(db.path())
        .args(["settings", "get", "movies"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let settings = parse_json(&output);

    let filterable = settings["filterableAttributes"]
        .as_array()
        .expect("filterableAttributes should be an array");
    let filterable_strs: Vec<&str> = filterable.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(
        filterable_strs.contains(&"year"),
        "filterableAttributes should include 'year'"
    );
    assert!(
        filterable_strs.contains(&"genres"),
        "filterableAttributes should include 'genres'"
    );
}

#[test]
fn test_settings_reset() {
    let db = fresh_db();
    create_index(db.path(), "movies");

    // Update settings first
    let settings_dir = TempDir::new().unwrap();
    let settings_path = settings_dir.path().join("settings.json");
    let settings_json = serde_json::json!({
        "filterableAttributes": ["year", "genres"],
        "sortableAttributes": ["year"]
    });
    std::fs::write(&settings_path, serde_json::to_string_pretty(&settings_json).unwrap())
        .expect("failed to write settings file");

    wily()
        .arg("--db")
        .arg(db.path())
        .args(["settings", "update", "movies"])
        .arg(&settings_path)
        .assert()
        .success();

    // Reset settings
    wily()
        .arg("--db")
        .arg(db.path())
        .args(["settings", "reset", "movies"])
        .assert()
        .success();

    // Verify settings are back to defaults
    let output = wily()
        .arg("--db")
        .arg(db.path())
        .args(["settings", "get", "movies"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let settings = parse_json(&output);

    let filterable = settings["filterableAttributes"]
        .as_array()
        .expect("filterableAttributes should be an array after reset");
    assert!(
        filterable.is_empty(),
        "filterableAttributes should be empty after reset"
    );
}

#[test]
fn test_settings_update_invalid_json() {
    let db = fresh_db();
    create_index(db.path(), "movies");

    let settings_dir = TempDir::new().unwrap();
    let settings_path = settings_dir.path().join("bad.json");
    std::fs::write(&settings_path, "{ this is not valid json !!!").unwrap();

    wily()
        .arg("--db")
        .arg(db.path())
        .args(["settings", "update", "movies"])
        .arg(&settings_path)
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("error").or(predicate::str::contains("Error")));
}

#[test]
fn test_settings_update_missing_file() {
    let db = fresh_db();
    create_index(db.path(), "movies");

    wily()
        .arg("--db")
        .arg(db.path())
        .args([
            "settings",
            "update",
            "movies",
            "/nonexistent_path_that_does_not_exist.json",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("error").or(predicate::str::contains("Error")));
}

#[test]
fn test_settings_get_nonexistent_index() {
    let db = fresh_db();

    wily()
        .arg("--db")
        .arg(db.path())
        .args(["settings", "get", "nonexistent"])
        .assert()
        .failure()
        .code(1);
}

// ─── Export command ──────────────────────────────────────────────────────────

#[test]
fn test_export_basic() {
    let db = fresh_db();
    create_index(db.path(), "movies");
    let _docs = add_sample_docs(db.path(), "movies");

    let export_dir = TempDir::new().expect("failed to create export dir");
    let export_path = export_dir.path().join("export_out");

    let output = wily()
        .arg("--db")
        .arg(db.path())
        .arg("export")
        .arg(&export_path)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let task = parse_json(&output);
    assert_eq!(
        task["type"].as_str().unwrap(),
        "export",
        "task type should be 'export'"
    );
    assert_eq!(
        task["status"].as_str().unwrap(),
        "succeeded",
        "task status should be 'succeeded'"
    );

    // Verify the export directory has content
    let movies_dir = export_path.join("movies");
    assert!(movies_dir.exists(), "export should create index directory");
    assert!(
        movies_dir.join("documents.json").exists(),
        "export should write documents.json"
    );
    assert!(
        movies_dir.join("settings.json").exists(),
        "export should write settings.json"
    );
}

#[test]
fn test_export_with_index_filter() {
    let db = fresh_db();
    create_index(db.path(), "movies");
    let _docs_movies = add_sample_docs(db.path(), "movies");

    create_index(db.path(), "books");
    let books_dir = TempDir::new().unwrap();
    let books_path = books_dir.path().join("books.json");
    let books = serde_json::json!([
        {"id": 1, "title": "Dune", "author": "Herbert"},
        {"id": 2, "title": "Neuromancer", "author": "Gibson"}
    ]);
    std::fs::write(&books_path, serde_json::to_string_pretty(&books).unwrap()).unwrap();
    wily()
        .arg("--db")
        .arg(db.path())
        .args(["doc", "add", "books"])
        .arg(&books_path)
        .assert()
        .success();

    let export_dir = TempDir::new().expect("failed to create export dir");
    let export_path = export_dir.path().join("filtered_export");

    wily()
        .arg("--db")
        .arg(db.path())
        .arg("export")
        .arg(&export_path)
        .args(["--indexes", "movies"])
        .assert()
        .success();

    // Only "movies" should be exported
    assert!(
        export_path.join("movies").exists(),
        "movies index should be exported"
    );
    assert!(
        !export_path.join("books").exists(),
        "books index should NOT be exported when filtering to 'movies'"
    );
}

#[test]
fn test_export_no_api_key_arg() {
    let db = fresh_db();
    create_index(db.path(), "movies");
    let _docs = add_sample_docs(db.path(), "movies");

    let export_dir = TempDir::new().expect("failed to create export dir");
    let export_path = export_dir.path().join("export_nokey");

    // Export should succeed without any api key argument
    wily()
        .arg("--db")
        .arg(db.path())
        .arg("export")
        .arg(&export_path)
        .assert()
        .success();
}

// ─── Error handling / edge cases ─────────────────────────────────────────────

#[test]
fn test_error_no_args() {
    wily()
        .assert()
        .failure()
        .code(2);
}

#[test]
fn test_error_unknown_subcommand() {
    let db = fresh_db();

    wily()
        .arg("--db")
        .arg(db.path())
        .arg("foobar")
        .assert()
        .failure()
        .code(2);
}

#[test]
fn test_error_index_get_nonexistent() {
    let db = fresh_db();

    wily()
        .arg("--db")
        .arg(db.path())
        .args(["index", "get", "nope"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("error").or(predicate::str::contains("Error")));
}

#[test]
fn test_error_doc_get_nonexistent_index() {
    let db = fresh_db();

    wily()
        .arg("--db")
        .arg(db.path())
        .args(["doc", "get", "nonexistent", "1"])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn test_error_doc_list_nonexistent_index() {
    let db = fresh_db();

    wily()
        .arg("--db")
        .arg(db.path())
        .args(["doc", "list", "nonexistent"])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn test_error_search_nonexistent_index() {
    let db = fresh_db();

    wily()
        .arg("--db")
        .arg(db.path())
        .args(["search", "nonexistent", "query"])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn test_error_settings_get_nonexistent_index_again() {
    let db = fresh_db();

    wily()
        .arg("--db")
        .arg(db.path())
        .args(["settings", "get", "nonexistent"])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn test_error_index_create_missing_uid() {
    let db = fresh_db();

    wily()
        .arg("--db")
        .arg(db.path())
        .args(["index", "create"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn test_error_index_update_missing_pk() {
    let db = fresh_db();

    wily()
        .arg("--db")
        .arg(db.path())
        .args(["index", "update", "movies"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn test_error_doc_add_missing_file_arg() {
    let db = fresh_db();

    wily()
        .arg("--db")
        .arg(db.path())
        .args(["doc", "add", "movies"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn test_error_doc_delete_batch_missing_ids() {
    let db = fresh_db();

    wily()
        .arg("--db")
        .arg(db.path())
        .args(["doc", "delete-batch", "movies"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn test_error_doc_delete_filter_missing_filter() {
    let db = fresh_db();

    wily()
        .arg("--db")
        .arg(db.path())
        .args(["doc", "delete-filter", "movies"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn test_error_facet_search_missing_facet_name() {
    let db = fresh_db();

    wily()
        .arg("--db")
        .arg(db.path())
        .args(["facet-search", "movies"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn test_db_flag_custom_path() {
    let db = fresh_db();
    let custom_db = db.path().join("custom_db_path");

    // Create an index at a custom db path
    wily()
        .arg("--db")
        .arg(&custom_db)
        .args(["index", "create", "testidx"])
        .assert()
        .success();

    // Verify we can retrieve it from the same custom path
    let output = wily()
        .arg("--db")
        .arg(&custom_db)
        .args(["index", "get", "testidx"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let idx = parse_json(&output);
    assert_eq!(idx["uid"].as_str().unwrap(), "testidx");
}

#[test]
fn test_help_flag() {
    wily()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Embedded Meilisearch CLI"));
}

#[test]
fn test_subcommand_help() {
    wily()
        .args(["index", "--help"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("create").and(predicate::str::contains("delete")),
        );
}
