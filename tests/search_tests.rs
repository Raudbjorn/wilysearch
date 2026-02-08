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

// ─── Federated multi-search ─────────────────────────────────────────────────

/// Helper: create both "movies" and "books" indexes for federated tests.
fn create_movies_and_books(ctx: &TestContext) {
    common::create_test_index(ctx, "movies");

    ctx.engine
        .create_index(&CreateIndexRequest {
            uid: "books".to_string(),
            primary_key: Some("id".to_string()),
        })
        .expect("failed to create books index");

    ctx.engine
        .add_or_replace_documents("books", &common::sample_books(), &AddDocumentsQuery::default())
        .expect("failed to add books");
}

#[test]
fn test_federated_multi_search_basic() {
    let ctx = TestContext::new();
    create_movies_and_books(&ctx);

    let request = MultiSearchRequest {
        queries: vec![
            MultiSearchQuery {
                index_uid: "movies".to_string(),
                search: SearchRequest {
                    q: Some("dark".to_string()),
                    ..Default::default()
                },
                federation_options: None,
            },
            MultiSearchQuery {
                index_uid: "books".to_string(),
                search: SearchRequest {
                    q: Some("dark".to_string()),
                    ..Default::default()
                },
                federation_options: None,
            },
        ],
        federation: Some(FederationSettings::default()),
    };

    let result = ctx.engine.multi_search(&request).expect("federated search failed");

    match result {
        MultiSearchResult::Federated(fed) => {
            assert!(!fed.hits.is_empty(), "expected merged hits from both indexes");
            // Default pagination uses offset/limit
            assert!(fed.offset.is_some() || fed.limit.is_some());
        }
        MultiSearchResult::PerIndex(_) => {
            panic!("expected Federated variant, got PerIndex");
        }
    }
}

#[test]
fn test_federated_multi_search_with_weights() {
    let ctx = TestContext::new();
    create_movies_and_books(&ctx);

    // Give books a very high weight so they rank above movies.
    let request = MultiSearchRequest {
        queries: vec![
            MultiSearchQuery {
                index_uid: "movies".to_string(),
                search: SearchRequest {
                    q: Some("the".to_string()),
                    ..Default::default()
                },
                federation_options: Some(FederationQueryOptions {
                    weight: Some(0.1),
                    query_position: None,
                }),
            },
            MultiSearchQuery {
                index_uid: "books".to_string(),
                search: SearchRequest {
                    q: Some("the".to_string()),
                    ..Default::default()
                },
                federation_options: Some(FederationQueryOptions {
                    weight: Some(10.0),
                    query_position: None,
                }),
            },
        ],
        federation: Some(FederationSettings::default()),
    };

    let result = ctx.engine.multi_search(&request).expect("federated search failed");

    match result {
        MultiSearchResult::Federated(fed) => {
            assert!(!fed.hits.is_empty(), "expected merged hits");
        }
        MultiSearchResult::PerIndex(_) => {
            panic!("expected Federated variant, got PerIndex");
        }
    }
}

#[test]
fn test_federated_multi_search_pagination() {
    let ctx = TestContext::new();
    create_movies_and_books(&ctx);

    // Use page-based pagination: page 1 with 3 hits per page.
    let request = MultiSearchRequest {
        queries: vec![
            MultiSearchQuery {
                index_uid: "movies".to_string(),
                search: SearchRequest {
                    q: Some("the".to_string()),
                    ..Default::default()
                },
                federation_options: None,
            },
            MultiSearchQuery {
                index_uid: "books".to_string(),
                search: SearchRequest {
                    q: Some("the".to_string()),
                    ..Default::default()
                },
                federation_options: None,
            },
        ],
        federation: Some(FederationSettings {
            page: Some(1),
            hits_per_page: Some(3),
            ..Default::default()
        }),
    };

    let result = ctx.engine.multi_search(&request).expect("federated search failed");

    match result {
        MultiSearchResult::Federated(fed) => {
            assert!(fed.hits.len() <= 3, "expected at most 3 hits, got {}", fed.hits.len());
            assert_eq!(fed.page, Some(1));
            assert_eq!(fed.hits_per_page, Some(3));
            assert!(fed.total_hits.is_some());
            assert!(fed.total_pages.is_some());
        }
        MultiSearchResult::PerIndex(_) => {
            panic!("expected Federated variant, got PerIndex");
        }
    }

    // Offset/limit pagination
    let request2 = MultiSearchRequest {
        queries: vec![
            MultiSearchQuery {
                index_uid: "movies".to_string(),
                search: SearchRequest {
                    q: Some("the".to_string()),
                    ..Default::default()
                },
                federation_options: None,
            },
        ],
        federation: Some(FederationSettings {
            limit: Some(2),
            offset: Some(1),
            ..Default::default()
        }),
    };

    let result2 = ctx.engine.multi_search(&request2).expect("federated search failed");

    match result2 {
        MultiSearchResult::Federated(fed) => {
            assert!(fed.hits.len() <= 2, "expected at most 2 hits, got {}", fed.hits.len());
            assert_eq!(fed.offset, Some(1));
            assert_eq!(fed.limit, Some(2));
            assert!(fed.estimated_total_hits.is_some());
        }
        MultiSearchResult::PerIndex(_) => {
            panic!("expected Federated variant, got PerIndex");
        }
    }
}

#[test]
fn test_federated_multi_search_single_index() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");

    let request = MultiSearchRequest {
        queries: vec![MultiSearchQuery {
            index_uid: "movies".to_string(),
            search: SearchRequest {
                q: Some("knight".to_string()),
                ..Default::default()
            },
            federation_options: None,
        }],
        federation: Some(FederationSettings::default()),
    };

    let result = ctx.engine.multi_search(&request).expect("federated search failed");

    match result {
        MultiSearchResult::Federated(fed) => {
            assert!(!fed.hits.is_empty(), "expected hits from single index");
        }
        MultiSearchResult::PerIndex(_) => {
            panic!("expected Federated variant, got PerIndex");
        }
    }
}

#[test]
fn test_non_federated_multi_search_returns_per_index() {
    let ctx = TestContext::new();
    create_movies_and_books(&ctx);

    let request = MultiSearchRequest {
        queries: vec![
            MultiSearchQuery {
                index_uid: "movies".to_string(),
                search: SearchRequest {
                    q: Some("dark".to_string()),
                    ..Default::default()
                },
                federation_options: None,
            },
            MultiSearchQuery {
                index_uid: "books".to_string(),
                search: SearchRequest {
                    q: Some("great".to_string()),
                    ..Default::default()
                },
                federation_options: None,
            },
        ],
        federation: None,
    };

    let result = ctx.engine.multi_search(&request).expect("multi_search failed");

    match result {
        MultiSearchResult::PerIndex(per_index) => {
            assert_eq!(per_index.results.len(), 2, "expected 2 per-index result sets");
            assert!(!per_index.results[0].hits.is_empty(), "movies should have hits for 'dark'");
            assert!(!per_index.results[1].hits.is_empty(), "books should have hits for 'great'");
        }
        MultiSearchResult::Federated(_) => {
            panic!("expected PerIndex variant, got Federated");
        }
    }
}
