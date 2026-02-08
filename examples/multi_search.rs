//! Multi-Search Example
//!
//! Demonstrates how to search across multiple indexes in a single call:
//!
//! 1. Creating two indexes (movies and books)
//! 2. Using `Meilisearch::multi_search()` to query both at once
//! 3. Inspecting per-index results from the combined response
//!
//! Multi-search is useful when a single user query should retrieve results
//! from different collections (e.g., a search bar that shows movies, books,
//! and articles simultaneously).

use wilysearch::core::{Meilisearch, MeilisearchOptions, MultiSearchQuery, SearchQuery};
use serde_json::json;

fn main() -> wilysearch::core::Result<()> {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let options = MeilisearchOptions {
        db_path: tmp_dir.path().to_path_buf(),
        ..Default::default()
    };
    let meili = Meilisearch::new(options)?;

    // ======================================================================
    // Create and populate two indexes
    // ======================================================================

    // -- Movies index --
    let movies_index = meili.create_index("movies", Some("id"))?;
    movies_index.add_documents(
        vec![
            json!({ "id": 1, "title": "Dune", "year": 2021, "type": "movie" }),
            json!({ "id": 2, "title": "Dune: Part Two", "year": 2024, "type": "movie" }),
            json!({ "id": 3, "title": "Blade Runner 2049", "year": 2017, "type": "movie" }),
            json!({ "id": 4, "title": "Arrival", "year": 2016, "type": "movie" }),
        ],
        None,
    )?;
    println!("Movies index: 4 documents added.");

    // -- Books index --
    let books_index = meili.create_index("books", Some("id"))?;
    books_index.add_documents(
        vec![
            json!({ "id": 1, "title": "Dune", "author": "Frank Herbert", "year": 1965, "type": "book" }),
            json!({ "id": 2, "title": "Neuromancer", "author": "William Gibson", "year": 1984, "type": "book" }),
            json!({ "id": 3, "title": "Foundation", "author": "Isaac Asimov", "year": 1951, "type": "book" }),
            json!({ "id": 4, "title": "Do Androids Dream of Electric Sheep?", "author": "Philip K. Dick", "year": 1968, "type": "book" }),
        ],
        None,
    )?;
    println!("Books index: 4 documents added.\n");

    // ======================================================================
    // Multi-search: query both indexes at once
    // ======================================================================
    println!("=== Multi-search: 'dune' across movies and books ===");
    let queries = vec![
        MultiSearchQuery {
            index_uid: "movies".to_string(),
            query: SearchQuery::new("dune").with_limit(5),
        },
        MultiSearchQuery {
            index_uid: "books".to_string(),
            query: SearchQuery::new("dune").with_limit(5),
        },
    ];

    let result = meili.multi_search(queries)?;

    for search_result in &result.results {
        println!(
            "[{}] {} hit(s):",
            search_result.index_uid,
            search_result.result.hits.len()
        );
        for hit in &search_result.result.hits {
            println!("  - {}", hit.document["title"]);
        }
    }

    // ======================================================================
    // Multi-search with different queries per index
    // ======================================================================
    println!("\n=== Multi-search: different queries per index ===");
    let queries = vec![
        MultiSearchQuery {
            index_uid: "movies".to_string(),
            query: SearchQuery::new("blade runner"),
        },
        MultiSearchQuery {
            index_uid: "books".to_string(),
            query: SearchQuery::new("androids dream"),
        },
    ];

    let result = meili.multi_search(queries)?;

    for search_result in &result.results {
        println!(
            "[{}] query='{}', {} hit(s):",
            search_result.index_uid,
            search_result.result.query,
            search_result.result.hits.len()
        );
        for hit in &search_result.result.hits {
            println!("  - {}", hit.document["title"]);
        }
    }

    // ======================================================================
    // Multi-search with ranking scores
    // ======================================================================
    println!("\n=== Multi-search with ranking scores ===");
    let queries = vec![
        MultiSearchQuery {
            index_uid: "movies".to_string(),
            query: SearchQuery::new("science fiction")
                .with_ranking_score(true)
                .with_limit(3),
        },
        MultiSearchQuery {
            index_uid: "books".to_string(),
            query: SearchQuery::new("science fiction")
                .with_ranking_score(true)
                .with_limit(3),
        },
    ];

    let result = meili.multi_search(queries)?;

    for search_result in &result.results {
        println!("[{}]:", search_result.index_uid);
        for hit in &search_result.result.hits {
            println!(
                "  - {} (score: {:.4})",
                hit.document["title"],
                hit.ranking_score.unwrap_or(0.0)
            );
        }
    }

    println!("\nDone.");
    Ok(())
}
