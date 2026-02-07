//! Settings Example
//!
//! Demonstrates the full lifecycle of index settings management:
//!
//! 1. Updating settings via the `Settings` builder (filterable, sortable, etc.)
//! 2. Reading settings back with `get_settings()`
//! 3. Using individual setting accessors (get/update/reset per setting)
//! 4. Configuring synonyms, stop words, and typo tolerance
//! 5. Resetting all settings to defaults

use wilysearch::core::{
    Meilisearch, MeilisearchOptions, MinWordSizeForTypos, Settings, TypoToleranceSettings,
};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

fn main() -> wilysearch::core::Result<()> {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let options = MeilisearchOptions {
        db_path: tmp_dir.path().to_path_buf(),
        ..Default::default()
    };
    let meili = Meilisearch::new(options)?;
    let index = meili.create_index("products", Some("id"))?;

    // Add sample documents so the index has a schema
    index.add_documents(
        vec![
            json!({ "id": 1, "name": "Laptop", "brand": "Acme", "price": 999, "category": "Electronics" }),
            json!({ "id": 2, "name": "Keyboard", "brand": "Acme", "price": 79, "category": "Electronics" }),
            json!({ "id": 3, "name": "Desk Chair", "brand": "ErgoMax", "price": 450, "category": "Furniture" }),
        ],
        None,
    )?;
    println!("3 products added.\n");

    // ======================================================================
    // 1. Bulk settings update via the Settings builder
    // ======================================================================
    println!("=== 1. Bulk settings update ===");

    let settings = Settings::new()
        .with_searchable_attributes(vec![
            "name".into(),
            "brand".into(),
            "category".into(),
        ])
        .with_filterable_attributes(vec![
            "price".into(),
            "category".into(),
            "brand".into(),
        ])
        .with_sortable_attributes(
            ["price".into(), "name".into()].into_iter().collect(),
        )
        .with_displayed_attributes(vec![
            "name".into(),
            "brand".into(),
            "price".into(),
            "category".into(),
        ]);

    index.update_settings(&settings)?;
    println!("Settings updated: searchable, filterable, sortable, displayed.");

    // ======================================================================
    // 2. Read settings back
    // ======================================================================
    println!("\n=== 2. Read current settings ===");

    let current = index.get_settings()?;
    println!(
        "Searchable attributes: {:?}",
        current.searchable_attributes
    );
    println!(
        "Filterable attributes: {:?}",
        current.filterable_attributes
    );
    println!("Sortable attributes:   {:?}", current.sortable_attributes);
    println!(
        "Displayed attributes:  {:?}",
        current.displayed_attributes
    );
    println!(
        "Ranking rules:         {:?}",
        current.ranking_rules
    );

    // ======================================================================
    // 3. Individual setting accessors
    // ======================================================================
    println!("\n=== 3. Individual setting accessors ===");

    // -- Filterable attributes --
    let filterable = index.get_filterable_attributes()?;
    println!("get_filterable_attributes: {:?}", filterable);

    // -- Sortable attributes --
    let sortable = index.get_sortable_attributes()?;
    println!("get_sortable_attributes:   {:?}", sortable);

    // -- Update stop words --
    let stop_words: BTreeSet<String> =
        ["the", "a", "an", "is", "at", "on"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    index.update_stop_words(stop_words)?;
    println!("\nStop words set: {:?}", index.get_stop_words()?);

    // -- Update synonyms --
    let mut synonyms = BTreeMap::new();
    synonyms.insert(
        "laptop".to_string(),
        vec!["notebook".to_string(), "computer".to_string()],
    );
    synonyms.insert(
        "chair".to_string(),
        vec!["seat".to_string(), "stool".to_string()],
    );
    index.update_synonyms(synonyms)?;
    println!("Synonyms set:   {:?}", index.get_synonyms()?);

    // ======================================================================
    // 4. Typo tolerance configuration
    // ======================================================================
    println!("\n=== 4. Typo tolerance ===");

    let typo_settings = TypoToleranceSettings {
        enabled: Some(true),
        min_word_size_for_typos: Some(MinWordSizeForTypos {
            one_typo: Some(4),
            two_typos: Some(8),
        }),
        disable_on_words: Some(
            ["Acme", "ErgoMax"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        ),
        disable_on_attributes: None,
        disable_on_numbers: None,
    };

    let settings = Settings::new().with_typo_tolerance(typo_settings);
    index.update_settings(&settings)?;
    println!("Typo tolerance configured:");
    let typo = index.get_typo_tolerance()?;
    if let Some(ref t) = typo {
        println!("  enabled: {:?}", t.enabled);
        if let Some(ref sizes) = t.min_word_size_for_typos {
            println!(
                "  min_word_size: one_typo={:?}, two_typos={:?}",
                sizes.one_typo, sizes.two_typos
            );
        }
        println!("  disable_on_words: {:?}", t.disable_on_words);
    }

    // ======================================================================
    // 5. Reset individual settings
    // ======================================================================
    println!("\n=== 5. Reset individual settings ===");

    index.reset_stop_words()?;
    println!("Stop words after reset:  {:?}", index.get_stop_words()?);

    index.reset_synonyms()?;
    println!("Synonyms after reset:    {:?}", index.get_synonyms()?);

    // ======================================================================
    // 6. Reset ALL settings
    // ======================================================================
    println!("\n=== 6. Reset all settings ===");

    index.reset_settings()?;
    let after_reset = index.get_settings()?;
    println!("Filterable after full reset: {:?}", after_reset.filterable_attributes);
    println!("Sortable after full reset:   {:?}", after_reset.sortable_attributes);
    println!("Stop words after full reset: {:?}", after_reset.stop_words);
    println!("Synonyms after full reset:   {:?}", after_reset.synonyms);

    println!("\nDone.");
    Ok(())
}
