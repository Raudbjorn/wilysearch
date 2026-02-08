//! Basic Search Example
//!
//! Demonstrates the core workflow of meilisearch-lib:
//! 1. Creating a Meilisearch instance with a temporary database
//! 2. Creating an index with a primary key
//! 3. Adding documents (movie data as JSON)
//! 4. Performing keyword searches with various options
//! 5. Using offset/limit pagination to browse results

use wilysearch::core::{Meilisearch, MeilisearchOptions, SearchQuery, Settings};
use serde_json::json;

fn main() -> wilysearch::core::Result<()> {
    // -- Setup: create an embedded Meilisearch instance in a temp directory --
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let options = MeilisearchOptions {
        db_path: tmp_dir.path().to_path_buf(),
        ..Default::default()
    };
    let meili = Meilisearch::new(options)?;
    println!("Meilisearch instance created at {:?}", tmp_dir.path());

    // -- Create an index with "id" as the primary key --
    let index = meili.create_index("movies", Some("id"))?;
    println!("Index 'movies' created.");

    // -- Configure filterable and sortable attributes --
    let settings = Settings::new()
        .with_searchable_attributes(vec![
            "title".into(),
            "overview".into(),
            "genres".into(),
        ])
        .with_filterable_attributes(vec!["year".into(), "genres".into()])
        .with_sortable_attributes(["year".into()].into_iter().collect());
    index.update_settings(&settings)?;
    println!("Settings configured (searchable, filterable, sortable).");

    // -- Add movie documents --
    let movies = vec![
        json!({
            "id": 1,
            "title": "The Shawshank Redemption",
            "overview": "Two imprisoned men bond over a number of years, finding solace and eventual redemption through acts of common decency.",
            "year": 1994,
            "genres": ["Drama"]
        }),
        json!({
            "id": 2,
            "title": "The Dark Knight",
            "overview": "When the menace known as the Joker wreaks havoc on Gotham, Batman must accept one of the greatest tests.",
            "year": 2008,
            "genres": ["Action", "Crime", "Drama"]
        }),
        json!({
            "id": 3,
            "title": "Inception",
            "overview": "A thief who steals corporate secrets through dream-sharing technology is given the task of planting an idea.",
            "year": 2010,
            "genres": ["Action", "Sci-Fi", "Thriller"]
        }),
        json!({
            "id": 4,
            "title": "Interstellar",
            "overview": "A team of explorers travel through a wormhole in space in an attempt to ensure humanity's survival.",
            "year": 2014,
            "genres": ["Adventure", "Drama", "Sci-Fi"]
        }),
        json!({
            "id": 5,
            "title": "The Matrix",
            "overview": "A computer hacker learns about the true nature of his reality and his role in the war against its controllers.",
            "year": 1999,
            "genres": ["Action", "Sci-Fi"]
        }),
        json!({
            "id": 6,
            "title": "Pulp Fiction",
            "overview": "The lives of two mob hitmen, a boxer, a gangster and his wife intertwine in four tales of violence and redemption.",
            "year": 1994,
            "genres": ["Crime", "Drama"]
        }),
    ];
    index.add_documents(movies, None)?;
    println!("6 movies added.\n");

    // -- Basic search --
    println!("=== Search: 'redemption' ===");
    let query = SearchQuery::new("redemption");
    let result = index.search(&query)?;
    println!(
        "Found {} hit(s) in {}ms",
        result.hits.len(),
        result.processing_time_ms
    );
    for hit in &result.hits {
        println!("  - {}", hit.document);
    }

    // -- Search with ranking score --
    println!("\n=== Search: 'dream' (with ranking scores) ===");
    let query = SearchQuery::new("dream").with_ranking_score(true);
    let result = index.search(&query)?;
    for hit in &result.hits {
        println!(
            "  - {} (score: {:.4})",
            hit.document["title"],
            hit.ranking_score.unwrap_or(0.0)
        );
    }

    // -- Search with a filter --
    println!("\n=== Search: 'action' filtered to year > 2005 ===");
    let query = SearchQuery::new("action").with_filter("year > 2005");
    let result = index.search(&query)?;
    println!("Hits after filtering:");
    for hit in &result.hits {
        println!(
            "  - {} ({})",
            hit.document["title"], hit.document["year"]
        );
    }

    // -- Pagination: offset/limit --
    println!("\n=== All documents, paginated (offset=0, limit=3) ===");
    let query = SearchQuery::match_all().with_limit(3).with_offset(0);
    let result = index.search(&query)?;
    println!("Page 1 ({} hits):", result.hits.len());
    for hit in &result.hits {
        println!("  - {}", hit.document["title"]);
    }

    println!("\n=== All documents, paginated (offset=3, limit=3) ===");
    let query = SearchQuery::match_all().with_limit(3).with_offset(3);
    let result = index.search(&query)?;
    println!("Page 2 ({} hits):", result.hits.len());
    for hit in &result.hits {
        println!("  - {}", hit.document["title"]);
    }

    // -- Search with sort --
    println!("\n=== Search: match all, sorted by year:asc ===");
    let query = SearchQuery::match_all()
        .with_sort(vec!["year:asc".into()])
        .with_limit(6);
    let result = index.search(&query)?;
    for hit in &result.hits {
        println!(
            "  - {} ({})",
            hit.document["title"], hit.document["year"]
        );
    }

    // -- Cleanup happens automatically when tmp_dir goes out of scope --
    println!("\nDone. Temp directory will be cleaned up automatically.");
    Ok(())
}
