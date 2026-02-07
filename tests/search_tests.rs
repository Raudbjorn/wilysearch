mod common;

use common::TestContext;
use serde_json::json;
use wilysearch::traits::*;
use wilysearch::types::*;

#[test]
fn test_basic_search() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");

    let result = ctx
        .engine
        .search("movies", &SearchRequest {
            q: Some("dark knight".to_string()),
            ..Default::default()
        })
        .expect("search failed");

    assert!(!result.hits.is_empty(), "expected at least one hit");
    let top_title = result.hits[0]["title"]
        .as_str()
        .expect("title should be a string");
    assert!(
        top_title.contains("Dark Knight"),
        "expected 'Dark Knight' in title, got: {top_title}"
    );
}

#[test]
fn test_search_with_filter() {
    let ctx = TestContext::new();
    common::create_configured_index(&ctx, "movies");

    let result = ctx
        .engine
        .search("movies", &SearchRequest {
            filter: Some(json!("year > 2000")),
            limit: Some(20),
            ..Default::default()
        })
        .expect("search failed");

    assert!(
        result.hits.len() >= 3,
        "expected at least 3 movies after 2000, got {}",
        result.hits.len()
    );

    for hit in &result.hits {
        let year = hit["year"].as_i64().expect("year should be a number");
        assert!(year > 2000, "expected year > 2000, got {year}");
    }
}

#[test]
fn test_search_with_sort() {
    let ctx = TestContext::new();
    common::create_configured_index(&ctx, "movies");

    let result = ctx
        .engine
        .search("movies", &SearchRequest {
            sort: Some(vec!["year:desc".to_string()]),
            limit: Some(20),
            ..Default::default()
        })
        .expect("search failed");

    assert!(!result.hits.is_empty());
    let years: Vec<i64> = result
        .hits
        .iter()
        .map(|h| h["year"].as_i64().unwrap())
        .collect();

    for window in years.windows(2) {
        assert!(
            window[0] >= window[1],
            "expected descending year order: {} >= {}",
            window[0],
            window[1]
        );
    }
}

#[test]
fn test_search_with_facets() {
    let ctx = TestContext::new();
    common::create_configured_index(&ctx, "movies");

    let result = ctx
        .engine
        .search("movies", &SearchRequest {
            facets: Some(vec!["genres".to_string()]),
            limit: Some(20),
            ..Default::default()
        })
        .expect("search failed");

    let facets = result
        .facet_distribution
        .as_ref()
        .expect("expected facet_distribution");
    assert!(
        facets.contains_key("genres"),
        "expected 'genres' in facet distribution"
    );
    let genre_facets = &facets["genres"];
    assert!(
        genre_facets.contains_key("Drama"),
        "expected 'Drama' in genre facets"
    );
    assert!(*genre_facets.get("Drama").unwrap() > 0);
}

#[test]
fn test_search_with_highlighting() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");

    let result = ctx
        .engine
        .search("movies", &SearchRequest {
            q: Some("dark".to_string()),
            attributes_to_highlight: Some(vec!["title".to_string()]),
            ..Default::default()
        })
        .expect("search failed");

    assert!(!result.hits.is_empty(), "expected at least one hit");
    // The formatted/highlighted content should be in the hit as _formatted
    let first_hit = &result.hits[0];
    if let Some(formatted) = first_hit.get("_formatted") {
        let formatted_title = formatted["title"]
            .as_str()
            .expect("formatted title should be string");
        assert!(
            formatted_title.contains("<em>") && formatted_title.contains("</em>"),
            "expected highlight tags in formatted title, got: {formatted_title}"
        );
    }
}

#[test]
fn test_search_page_pagination() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");

    let result = ctx
        .engine
        .search("movies", &SearchRequest {
            page: Some(1),
            hits_per_page: Some(3),
            ..Default::default()
        })
        .expect("search failed");

    assert_eq!(result.hits.len(), 3);
    assert_eq!(result.page, Some(1));
    assert_eq!(result.hits_per_page, Some(3));
    assert_eq!(result.total_hits, Some(10));
    assert_eq!(result.total_pages, Some(4)); // ceil(10/3)
}

#[test]
fn test_search_offset_pagination() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");

    let result = ctx
        .engine
        .search("movies", &SearchRequest {
            offset: Some(2),
            limit: Some(3),
            ..Default::default()
        })
        .expect("search failed");

    assert_eq!(result.hits.len(), 3);
    assert_eq!(result.offset, Some(2));
    assert_eq!(result.limit, Some(3));
    assert_eq!(result.estimated_total_hits, Some(10));
}

#[test]
fn test_search_matching_strategy_all() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");

    let result = ctx
        .engine
        .search("movies", &SearchRequest {
            q: Some("dark knight".to_string()),
            matching_strategy: Some(MatchingStrategy::All),
            ..Default::default()
        })
        .expect("search failed");

    for hit in &result.hits {
        let title = hit["title"].as_str().unwrap().to_lowercase();
        assert!(
            title.contains("dark") && title.contains("knight"),
            "with All strategy, expected both 'dark' and 'knight' in title, got: {title}"
        );
    }
}

#[test]
fn test_search_empty_query() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");

    let result = ctx
        .engine
        .search("movies", &SearchRequest {
            limit: Some(20),
            ..Default::default()
        })
        .expect("search failed");

    assert_eq!(result.hits.len(), 10);
}

#[test]
fn test_search_no_results() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");

    let result = ctx
        .engine
        .search("movies", &SearchRequest {
            q: Some("xyzzy_nonexistent_gobbledygook".to_string()),
            ..Default::default()
        })
        .expect("search failed");

    assert!(
        result.hits.is_empty(),
        "expected no results for nonsense query, got {}",
        result.hits.len()
    );
}

#[test]
fn test_search_attributes_to_retrieve() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");

    let result = ctx
        .engine
        .search("movies", &SearchRequest {
            q: Some("inception".to_string()),
            attributes_to_retrieve: Some(vec!["title".to_string(), "year".to_string()]),
            ..Default::default()
        })
        .expect("search failed");

    assert!(!result.hits.is_empty());
    let doc = &result.hits[0];
    let obj = doc.as_object().expect("doc should be object");
    assert!(obj.contains_key("title"));
    assert!(obj.contains_key("year"));
    assert!(!obj.contains_key("genres"), "genres should not be returned");
    assert!(!obj.contains_key("rating"), "rating should not be returned");
}

#[test]
fn test_search_attributes_to_search_on() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");

    let result = ctx
        .engine
        .search("movies", &SearchRequest {
            q: Some("Drama".to_string()),
            attributes_to_search_on: Some(vec!["title".to_string()]),
            ..Default::default()
        })
        .expect("search failed");

    assert!(
        result.hits.is_empty(),
        "expected no results when searching 'Drama' only in title field, got {}",
        result.hits.len()
    );
}

#[test]
fn test_facet_search() {
    let ctx = TestContext::new();
    common::create_configured_index(&ctx, "movies");

    let result = ctx
        .engine
        .facet_search("movies", &FacetSearchRequest {
            facet_name: "genres".to_string(),
            facet_query: Some("act".to_string()),
            q: None,
            filter: None,
            matching_strategy: None,
            attributes_to_search_on: None,
        })
        .expect("facet search failed");

    let values: Vec<&str> = result.facet_hits.iter().map(|h| h.value.as_str()).collect();
    assert!(
        values.iter().any(|v| v.to_lowercase().starts_with("act")),
        "expected a facet hit starting with 'act', got: {values:?}"
    );
}

#[test]
fn test_search_distinct() {
    let ctx = TestContext::new();
    common::create_configured_index(&ctx, "movies");

    // Set year as distinct attribute
    ctx.engine
        .update_settings("movies", &Settings {
            distinct_attribute: Some("year".to_string()),
            ..Default::default()
        })
        .expect("failed to update settings");

    let result = ctx
        .engine
        .search("movies", &SearchRequest {
            distinct: Some("year".to_string()),
            limit: Some(20),
            ..Default::default()
        })
        .expect("search failed");

    let years: Vec<i64> = result
        .hits
        .iter()
        .map(|h| h["year"].as_i64().unwrap())
        .collect();
    let unique_years: std::collections::HashSet<i64> = years.iter().copied().collect();
    assert_eq!(
        years.len(),
        unique_years.len(),
        "expected distinct years, got duplicates: {years:?}"
    );
}

#[test]
fn test_search_crop() {
    let ctx = TestContext::new();
    // Create a books index with long text
    ctx.engine
        .create_index(&CreateIndexRequest {
            uid: "books".to_string(),
            primary_key: Some("id".to_string()),
        })
        .expect("failed to create index");

    ctx.engine
        .add_or_replace_documents(
            "books",
            &[serde_json::json!({
                "id": 1,
                "title": "Test Book",
                "description": "This is a long description about the dark mysterious ways of the world that goes on and on for many words to test cropping"
            })],
            &AddDocumentsQuery::default(),
        )
        .expect("failed to add documents");

    let result = ctx
        .engine
        .search("books", &SearchRequest {
            q: Some("dark".to_string()),
            attributes_to_crop: Some(vec!["description".to_string()]),
            crop_length: Some(5),
            ..Default::default()
        })
        .expect("search failed");

    assert!(!result.hits.is_empty());
    if let Some(formatted) = result.hits[0].get("_formatted") {
        let cropped = formatted["description"]
            .as_str()
            .expect("cropped description should be a string");
        assert!(
            cropped.len() < 120,
            "expected cropped text to be shorter, got {} chars: {cropped}",
            cropped.len()
        );
    }
}
