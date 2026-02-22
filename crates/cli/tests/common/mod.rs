use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;

/// Build a `wily` command ready for argument chaining.
pub fn wily() -> Command {
    cargo_bin_cmd!("wily")
}

/// Create a fresh LMDB database directory and return the `TempDir` handle.
pub fn fresh_db() -> TempDir {
    TempDir::new().expect("failed to create temp db dir")
}

/// Run `wily --db <db> index create <uid>` and assert success.
pub fn create_index(db: &Path, uid: &str) {
    wily()
        .arg("--db")
        .arg(db)
        .args(["index", "create", uid])
        .assert()
        .success();
}

/// Write sample movie documents to a temp JSON file, add them to an index, and
/// return the directory handle so the file lives long enough.
pub fn add_sample_docs(db: &Path, uid: &str) -> TempDir {
    let doc_dir = TempDir::new().expect("failed to create doc dir");
    let doc_path = doc_dir.path().join("docs.json");

    let docs = serde_json::json!([
        {"id": 1, "title": "The Matrix", "year": 1999, "genres": ["sci-fi", "action"]},
        {"id": 2, "title": "Inception", "year": 2010, "genres": ["sci-fi", "thriller"]},
        {"id": 3, "title": "Interstellar", "year": 2014, "genres": ["sci-fi", "drama"]}
    ]);
    std::fs::write(&doc_path, serde_json::to_string_pretty(&docs).unwrap())
        .expect("failed to write doc file");

    wily()
        .arg("--db")
        .arg(db)
        .args(["doc", "add", uid])
        .arg(&doc_path)
        .assert()
        .success();

    doc_dir
}

/// Parse stdout bytes as JSON `Value`.
pub fn parse_json(stdout: &[u8]) -> Value {
    serde_json::from_slice(stdout).expect("stdout should be valid JSON")
}
