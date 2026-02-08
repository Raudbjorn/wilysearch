use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

// ---- Helpers ----------------------------------------------------------------

fn wily() -> Command {
    cargo_bin_cmd!("wily")
}

/// Create a movies index with filterable/sortable attributes and sample docs.
fn setup_searchable_movies(tmp: &TempDir) -> &std::path::Path {
    let db = tmp.path();

    // Create index
    wily()
        .arg("--db")
        .arg(db)
        .args(["index", "create", "movies", "--primary-key", "id"])
        .assert()
        .success();

    // Configure settings: make genres filterable and year sortable
    let settings_file = tmp.path().join("settings.json");
    std::fs::write(
        &settings_file,
        r#"{
            "filterableAttributes": ["genres", "year"],
            "sortableAttributes": ["year", "title"]
        }"#,
    )
    .unwrap();
    wily()
        .arg("--db")
        .arg(db)
        .args(["settings", "update", "movies"])
        .arg(&settings_file)
        .assert()
        .success();

    // Add documents
    let doc_file = tmp.path().join("movies.json");
    std::fs::write(
        &doc_file,
        r#"[
            {"id":1,"title":"The Matrix","year":1999,"genres":["Sci-Fi","Action"]},
            {"id":2,"title":"Inception","year":2010,"genres":["Sci-Fi","Thriller"]},
            {"id":3,"title":"Pulp Fiction","year":1994,"genres":["Crime","Drama"]},
            {"id":4,"title":"The Dark Knight","year":2008,"genres":["Action","Crime"]},
            {"id":5,"title":"Interstellar","year":2014,"genres":["Sci-Fi","Drama"]}
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

/// Run a search command, assert success, and return parsed JSON.
fn search_ok(db: &std::path::Path, args: &[&str]) -> Value {
    let out = wily()
        .arg("--db")
        .arg(db)
        .arg("search")
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("stdout is valid JSON")
}

/// Run a facet-search command, assert success, and return parsed JSON.
fn facet_search_ok(db: &std::path::Path, args: &[&str]) -> Value {
    let out = wily()
        .arg("--db")
        .arg(db)
        .arg("facet-search")
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("stdout is valid JSON")
}

// ---- Basic search -----------------------------------------------------------

#[test]
fn test_search_basic() {
    let tmp = TempDir::new().unwrap();
    let db = setup_searchable_movies(&tmp);

    let json = search_ok(db, &["movies", "matrix"]);

    let hits = json["hits"].as_array().expect("hits is an array");
    assert!(!hits.is_empty(), "expected at least one hit for 'matrix'");

    // The Matrix should be in the results
    let titles: Vec<&str> = hits
        .iter()
        .filter_map(|h| h["title"].as_str())
        .collect();
    assert!(
        titles.iter().any(|t| t.contains("Matrix")),
        "expected 'The Matrix' in hits, got: {titles:?}"
    );

    assert_eq!(json["query"].as_str(), Some("matrix"));
    assert!(
        json["processingTimeMs"].is_number(),
        "expected processingTimeMs to be a number"
    );
}

#[test]
fn test_search_empty_query() {
    let tmp = TempDir::new().unwrap();
    let db = setup_searchable_movies(&tmp);

    // No query argument => returns all documents (explicit limit needed; the
    // engine defaults to limit=0 when no query string is provided).
    let json = search_ok(db, &["movies", "--limit", "20"]);

    let hits = json["hits"].as_array().expect("hits is an array");
    assert_eq!(hits.len(), 5, "expected all 5 documents");
}

#[test]
fn test_search_no_results() {
    let tmp = TempDir::new().unwrap();
    let db = setup_searchable_movies(&tmp);

    let json = search_ok(db, &["movies", "zzzzxxyy"]);

    let hits = json["hits"].as_array().expect("hits is an array");
    assert!(hits.is_empty(), "expected no hits for nonsense query");
}

#[test]
fn test_search_with_limit() {
    let tmp = TempDir::new().unwrap();
    let db = setup_searchable_movies(&tmp);

    let json = search_ok(db, &["movies", "--limit", "2"]);

    let hits = json["hits"].as_array().expect("hits is an array");
    assert!(hits.len() <= 2, "expected at most 2 hits, got {}", hits.len());
}

#[test]
fn test_search_with_offset() {
    let tmp = TempDir::new().unwrap();
    let db = setup_searchable_movies(&tmp);

    let json = search_ok(db, &["movies", "--offset", "2", "--limit", "2"]);

    let hits = json["hits"].as_array().expect("hits is an array");
    assert!(!hits.is_empty(), "expected hits with offset 2");
    assert!(hits.len() <= 2, "expected at most 2 hits, got {}", hits.len());
    assert_eq!(
        json["offset"].as_u64(),
        Some(2),
        "expected offset to be 2 in response"
    );
}

// ---- Search with filter -----------------------------------------------------

#[test]
fn test_search_with_filter() {
    let tmp = TempDir::new().unwrap();
    let db = setup_searchable_movies(&tmp);

    let json = search_ok(db, &["movies", "--limit", "20", "--filter", "year > 2005"]);

    let hits = json["hits"].as_array().expect("hits is an array");
    assert!(!hits.is_empty(), "expected some hits with year > 2005");

    for hit in hits {
        let year = hit["year"]
            .as_u64()
            .expect("each hit should have a numeric year");
        assert!(year > 2005, "expected year > 2005, got {year}");
    }
}

// ---- Search with sort -------------------------------------------------------

#[test]
fn test_search_with_sort() {
    let tmp = TempDir::new().unwrap();
    let db = setup_searchable_movies(&tmp);

    let json = search_ok(db, &["movies", "--limit", "20", "--sort", "year:asc"]);

    let hits = json["hits"].as_array().expect("hits is an array");
    assert!(hits.len() >= 2, "need at least 2 hits to verify sorting");

    let years: Vec<u64> = hits
        .iter()
        .filter_map(|h| h["year"].as_u64())
        .collect();

    for window in years.windows(2) {
        assert!(
            window[0] <= window[1],
            "expected ascending year order, got {years:?}"
        );
    }
}

// ---- Search with facets -----------------------------------------------------

#[test]
fn test_search_with_facets() {
    let tmp = TempDir::new().unwrap();
    let db = setup_searchable_movies(&tmp);

    let json = search_ok(db, &["movies", "--limit", "20", "--facets", "genres"]);

    let facets = &json["facetDistribution"];
    assert!(facets.is_object(), "expected facetDistribution object");
    assert!(
        facets["genres"].is_object(),
        "expected genres key in facetDistribution"
    );
}

// ---- Search with fields -----------------------------------------------------

#[test]
fn test_search_with_fields() {
    let tmp = TempDir::new().unwrap();
    let db = setup_searchable_movies(&tmp);

    let json = search_ok(db, &["movies", "", "--limit", "20", "--fields", "id,title"]);

    let hits = json["hits"].as_array().expect("hits is an array");
    assert!(!hits.is_empty(), "expected some hits");

    for hit in hits {
        assert!(hit["id"].is_number(), "hit should have id");
        assert!(hit["title"].is_string(), "hit should have title");
        assert!(
            hit.get("year").is_none() || hit["year"].is_null(),
            "hit should NOT have year when fields=id,title"
        );
    }
}

// ---- Search with ranking score ----------------------------------------------

#[test]
fn test_search_with_ranking_score() {
    let tmp = TempDir::new().unwrap();
    let db = setup_searchable_movies(&tmp);

    let json = search_ok(db, &["movies", "matrix", "--show-ranking-score"]);

    let hits = json["hits"].as_array().expect("hits is an array");
    assert!(!hits.is_empty(), "expected at least one hit");

    for hit in hits {
        assert!(
            hit["_rankingScore"].is_number(),
            "expected _rankingScore on each hit, got: {hit}"
        );
    }
}

// ---- Search with matching strategy ------------------------------------------

#[test]
fn test_search_matching_strategy_all() {
    let tmp = TempDir::new().unwrap();
    let db = setup_searchable_movies(&tmp);

    // "all" requires all query terms to match -- may return no hits, but must succeed
    let json = search_ok(
        db,
        &["movies", "matrix inception", "--matching-strategy", "all"],
    );

    assert!(json["hits"].is_array(), "hits should be an array");
}

#[test]
fn test_search_matching_strategy_last() {
    let tmp = TempDir::new().unwrap();
    let db = setup_searchable_movies(&tmp);

    let json = search_ok(
        db,
        &["movies", "matrix inception", "--matching-strategy", "last"],
    );

    assert!(json["hits"].is_array(), "hits should be an array");
}

#[test]
fn test_search_matching_strategy_frequency() {
    let tmp = TempDir::new().unwrap();
    let db = setup_searchable_movies(&tmp);

    let json = search_ok(
        db,
        &[
            "movies",
            "matrix",
            "--matching-strategy",
            "frequency",
        ],
    );

    assert!(json["hits"].is_array(), "hits should be an array");
}

// ---- Facet search -----------------------------------------------------------

#[test]
fn test_facet_search() {
    let tmp = TempDir::new().unwrap();
    let db = setup_searchable_movies(&tmp);

    let json = facet_search_ok(db, &["movies", "--facet-name", "genres"]);

    let facet_hits = json["facetHits"]
        .as_array()
        .expect("facetHits should be an array");
    assert!(!facet_hits.is_empty(), "expected facet hits for genres");
    assert!(
        json["processingTimeMs"].is_number(),
        "expected processingTimeMs"
    );
}

#[test]
fn test_facet_search_with_query() {
    let tmp = TempDir::new().unwrap();
    let db = setup_searchable_movies(&tmp);

    let json = facet_search_ok(
        db,
        &["movies", "--facet-name", "genres", "--facet-query", "sci"],
    );

    let facet_hits = json["facetHits"]
        .as_array()
        .expect("facetHits should be an array");

    let values: Vec<&str> = facet_hits
        .iter()
        .filter_map(|fh| fh["value"].as_str())
        .collect();
    assert!(
        values.iter().any(|v| v.to_lowercase().contains("sci")),
        "expected 'Sci-Fi' in facet hits, got: {values:?}"
    );
}

#[test]
fn test_facet_search_with_main_query() {
    let tmp = TempDir::new().unwrap();
    let db = setup_searchable_movies(&tmp);

    let json = facet_search_ok(
        db,
        &["movies", "--facet-name", "genres", "--query", "matrix"],
    );

    assert!(
        json["facetHits"].is_array(),
        "facetHits should be an array"
    );
}

#[test]
fn test_facet_search_with_filter() {
    let tmp = TempDir::new().unwrap();
    let db = setup_searchable_movies(&tmp);

    let json = facet_search_ok(
        db,
        &[
            "movies",
            "--facet-name",
            "genres",
            "--filter",
            "year > 2005",
        ],
    );

    assert!(
        json["facetHits"].is_array(),
        "facetHits should be an array"
    );
}

// ---- Combined options -------------------------------------------------------

#[test]
fn test_search_combined_options() {
    let tmp = TempDir::new().unwrap();
    let db = setup_searchable_movies(&tmp);

    let json = search_ok(
        db,
        &[
            "movies",
            "--limit",
            "3",
            "--filter",
            "year > 1995",
            "--sort",
            "year:desc",
            "--facets",
            "genres",
            "--fields",
            "id,title,year",
        ],
    );

    // Verify limit
    let hits = json["hits"].as_array().expect("hits is an array");
    assert!(hits.len() <= 3, "expected at most 3 hits, got {}", hits.len());

    // Verify filter: all years > 1995
    for hit in hits {
        let year = hit["year"]
            .as_u64()
            .expect("each hit should have a numeric year");
        assert!(year > 1995, "expected year > 1995, got {year}");
    }

    // Verify sort: descending by year
    let years: Vec<u64> = hits
        .iter()
        .filter_map(|h| h["year"].as_u64())
        .collect();
    for window in years.windows(2) {
        assert!(
            window[0] >= window[1],
            "expected descending year order, got {years:?}"
        );
    }

    // Verify facets
    let facets = &json["facetDistribution"];
    assert!(facets.is_object(), "expected facetDistribution object");
    assert!(
        facets["genres"].is_object(),
        "expected genres in facetDistribution"
    );

    // Verify fields: should have id, title, year but NOT genres
    for hit in hits {
        assert!(hit["id"].is_number(), "hit should have id");
        assert!(hit["title"].is_string(), "hit should have title");
        assert!(hit["year"].is_number(), "hit should have year");
        assert!(
            hit.get("genres").is_none() || hit["genres"].is_null(),
            "hit should NOT have genres when fields=id,title,year"
        );
    }
}
