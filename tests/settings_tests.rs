mod common;

use common::TestContext;
use wilysearch::traits::*;
use wilysearch::types::*;
use std::collections::HashMap;

#[test]
fn test_get_default_settings() {
    let ctx = TestContext::new();
    ctx.engine
        .create_index(&CreateIndexRequest {
            uid: "test".to_string(),
            primary_key: Some("id".to_string()),
        })
        .expect("failed to create index");

    let settings = ctx
        .engine
        .get_settings("test")
        .expect("failed to get settings");

    // Default ranking rules should be present
    let rules = settings.ranking_rules.expect("ranking_rules should be set");
    assert!(!rules.is_empty(), "default ranking rules should not be empty");

    // Default: filterable and sortable should be empty
    let filterable = settings
        .filterable_attributes
        .expect("filterable should be set");
    assert!(filterable.is_empty());

    let sortable = settings
        .sortable_attributes
        .expect("sortable should be set");
    assert!(sortable.is_empty());

    // Typo tolerance should be enabled by default
    let typo = settings.typo_tolerance.expect("typo_tolerance should be set");
    assert_eq!(typo.enabled, Some(true));
}

#[test]
fn test_update_settings_bulk() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");

    let settings = Settings {
        searchable_attributes: Some(vec!["title".to_string()]),
        filterable_attributes: Some(vec!["year".to_string(), "genres".to_string()]),
        sortable_attributes: Some(vec!["year".to_string(), "rating".to_string()]),
        stop_words: Some(vec!["the".to_string(), "a".to_string()]),
        ..Default::default()
    };

    ctx.engine
        .update_settings("movies", &settings)
        .expect("failed to update settings");

    let got = ctx
        .engine
        .get_settings("movies")
        .expect("failed to get settings");

    assert_eq!(
        got.searchable_attributes,
        Some(vec!["title".to_string()])
    );

    let mut filterable = got.filterable_attributes.expect("filterable should be set");
    filterable.sort();
    assert_eq!(filterable, vec!["genres".to_string(), "year".to_string()]);

    let mut sortable = got.sortable_attributes.expect("sortable should be set");
    sortable.sort();
    assert_eq!(sortable, vec!["rating".to_string(), "year".to_string()]);

    let stop = got.stop_words.expect("stop_words should be set");
    assert!(stop.contains(&"the".to_string()));
    assert!(stop.contains(&"a".to_string()));
}

#[test]
fn test_reset_settings() {
    let ctx = TestContext::new();
    ctx.engine
        .create_index(&CreateIndexRequest {
            uid: "test".to_string(),
            primary_key: Some("id".to_string()),
        })
        .expect("failed to create index");

    let settings = Settings {
        filterable_attributes: Some(vec!["year".to_string()]),
        stop_words: Some(vec!["the".to_string()]),
        ..Default::default()
    };
    ctx.engine
        .update_settings("test", &settings)
        .expect("failed to update settings");

    // Verify they were set
    let got = ctx.engine.get_settings("test").expect("failed to get settings");
    assert!(!got.filterable_attributes.as_ref().unwrap().is_empty());

    // Reset
    ctx.engine
        .reset_settings("test")
        .expect("failed to reset settings");

    let after = ctx
        .engine
        .get_settings("test")
        .expect("failed to get settings after reset");
    let filterable = after
        .filterable_attributes
        .expect("filterable should be set");
    assert!(filterable.is_empty(), "filterable should be empty after reset");
}

#[test]
fn test_individual_searchable_attributes() {
    let ctx = TestContext::new();
    common::create_test_index(&ctx, "movies");

    ctx.engine
        .update_searchable_attributes("movies", &["title".to_string()])
        .expect("failed to update searchable");

    let got = ctx
        .engine
        .get_searchable_attributes("movies")
        .expect("failed to get searchable");
    assert_eq!(got, vec!["title".to_string()]);

    // Reset
    ctx.engine
        .reset_searchable_attributes("movies")
        .expect("failed to reset searchable");
    let after = ctx
        .engine
        .get_searchable_attributes("movies")
        .expect("failed to get searchable after reset");
    assert!(after.len() > 1, "expected multiple searchable fields after reset");
}

#[test]
fn test_individual_filterable_attributes() {
    let ctx = TestContext::new();
    ctx.engine
        .create_index(&CreateIndexRequest {
            uid: "test".to_string(),
            primary_key: Some("id".to_string()),
        })
        .expect("failed to create index");

    ctx.engine
        .update_filterable_attributes("test", &["year".to_string(), "genre".to_string()])
        .expect("failed to update filterable");

    let mut got = ctx
        .engine
        .get_filterable_attributes("test")
        .expect("failed to get filterable");
    got.sort();
    assert_eq!(got, vec!["genre".to_string(), "year".to_string()]);

    // Reset
    ctx.engine
        .reset_filterable_attributes("test")
        .expect("failed to reset filterable");
    let after = ctx
        .engine
        .get_filterable_attributes("test")
        .expect("failed to get filterable after reset");
    assert!(after.is_empty());
}

#[test]
fn test_individual_sortable_attributes() {
    let ctx = TestContext::new();
    ctx.engine
        .create_index(&CreateIndexRequest {
            uid: "test".to_string(),
            primary_key: Some("id".to_string()),
        })
        .expect("failed to create index");

    ctx.engine
        .update_sortable_attributes("test", &["price".to_string(), "date".to_string()])
        .expect("failed to update sortable");

    let mut got = ctx
        .engine
        .get_sortable_attributes("test")
        .expect("failed to get sortable");
    got.sort();
    assert_eq!(got, vec!["date".to_string(), "price".to_string()]);

    // Reset
    ctx.engine
        .reset_sortable_attributes("test")
        .expect("failed to reset sortable");
    let after = ctx
        .engine
        .get_sortable_attributes("test")
        .expect("failed to get sortable after reset");
    assert!(after.is_empty());
}

#[test]
fn test_individual_stop_words() {
    let ctx = TestContext::new();
    ctx.engine
        .create_index(&CreateIndexRequest {
            uid: "test".to_string(),
            primary_key: Some("id".to_string()),
        })
        .expect("failed to create index");

    let words = vec!["the".to_string(), "a".to_string(), "an".to_string()];
    ctx.engine
        .update_stop_words("test", &words)
        .expect("failed to update stop words");

    let mut got = ctx
        .engine
        .get_stop_words("test")
        .expect("failed to get stop words");
    got.sort();
    let mut expected = words.clone();
    expected.sort();
    assert_eq!(got, expected);

    // Reset
    ctx.engine
        .reset_stop_words("test")
        .expect("failed to reset stop words");
    let after = ctx
        .engine
        .get_stop_words("test")
        .expect("failed to get stop words after reset");
    assert!(after.is_empty(), "stop words should be empty after reset");
}

#[test]
fn test_individual_synonyms() {
    let ctx = TestContext::new();
    ctx.engine
        .create_index(&CreateIndexRequest {
            uid: "test".to_string(),
            primary_key: Some("id".to_string()),
        })
        .expect("failed to create index");

    let mut synonyms = HashMap::new();
    synonyms.insert(
        "car".to_string(),
        vec!["automobile".to_string(), "vehicle".to_string()],
    );
    synonyms.insert(
        "phone".to_string(),
        vec!["telephone".to_string(), "mobile".to_string()],
    );

    ctx.engine
        .update_synonyms("test", &synonyms)
        .expect("failed to update synonyms");

    let got = ctx
        .engine
        .get_synonyms("test")
        .expect("failed to get synonyms");
    assert!(got.contains_key("car"));
    assert!(got.contains_key("phone"));

    // Reset
    ctx.engine
        .reset_synonyms("test")
        .expect("failed to reset synonyms");
    let after = ctx
        .engine
        .get_synonyms("test")
        .expect("failed to get synonyms after reset");
    assert!(after.is_empty(), "synonyms should be empty after reset");
}
