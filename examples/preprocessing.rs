//! Preprocessing Pipeline Example
//!
//! Demonstrates the query preprocessing pipeline provided by meilisearch-lib:
//!
//! 1. Building a `QueryPipeline` with `TypoCorrector` and `SynonymMap`
//! 2. Processing queries through the pipeline
//! 3. Inspecting corrections, expansions, and generated hints
//! 4. Using the `QueryPipelineBuilder` for a fluent configuration API
//! 5. Loading synonyms from TOML strings
//!
//! The preprocessing pipeline runs independently of the search engine and is
//! useful for normalizing, correcting, and expanding user queries before they
//! are sent to Meilisearch for execution.

use wilysearch::core::{
    QueryPipeline, QueryPipelineBuilder, SynonymMap, TypoConfig, TypoCorrector,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ======================================================================
    // 1. Manual pipeline construction
    // ======================================================================
    println!("=== 1. Manual pipeline construction ===\n");

    // -- Create a typo corrector with a small dictionary --
    let mut typo_corrector = TypoCorrector::new(TypoConfig::default())?;
    // Load some words so the corrector can suggest corrections.
    // In production, you would load a full dictionary file.
    typo_corrector.load_dictionary([
        ("search", 1000),
        ("engine", 800),
        ("query", 600),
        ("document", 500),
        ("index", 400),
        ("filter", 300),
        ("attribute", 200),
        ("ranking", 150),
        ("synonym", 100),
        ("correction", 90),
        ("recovery", 80),
        ("health", 70),
    ]);
    println!("Typo corrector loaded with {} words.", 12);

    // -- Create a synonym map --
    let mut synonym_map = SynonymMap::new();
    synonym_map.add_multi_way(&["search", "query", "lookup"]);
    synonym_map.add_multi_way(&["document", "doc", "record"]);
    synonym_map.add_one_way("engine", &["motor", "system"]);
    println!(
        "Synonym map: {} multi-way groups, {} one-way mappings.\n",
        synonym_map.multi_way_group_count(),
        synonym_map.one_way_mapping_count()
    );

    // -- Build the pipeline --
    let pipeline = QueryPipeline::new(typo_corrector, synonym_map);

    // -- Process queries --
    let test_queries = [
        "search engine",
        "serach engne",    // typos
        "document filter",
        "query ranking",
    ];

    for raw_query in &test_queries {
        let processed = pipeline.process(raw_query);

        println!("Input:     \"{}\"", processed.original);
        println!("Corrected: \"{}\"", processed.corrected);

        if processed.has_corrections() {
            println!("Corrections:");
            for c in &processed.corrections {
                println!(
                    "  '{}' -> '{}' (distance: {}, confidence: {:.2})",
                    c.original, c.corrected, c.edit_distance, c.confidence
                );
            }
        }

        if processed.has_expansions() {
            println!("Expanded:  {}", processed.expanded.to_expanded_string());
        }

        println!(
            "Embedding: \"{}\"",
            processed.text_for_embedding
        );

        let hints = processed.generate_hints();
        if !hints.is_empty() {
            println!("Hints:");
            for hint in &hints {
                println!("  {}", hint);
            }
        }

        println!(
            "Processing time: {} us\n",
            processed.processing_time_us
        );
    }

    // ======================================================================
    // 2. QueryPipelineBuilder (fluent API)
    // ======================================================================
    println!("=== 2. QueryPipelineBuilder ===\n");

    let pipeline = QueryPipelineBuilder::new()
        .min_word_size_one_typo(5)
        .min_word_size_two_typos(9)
        .word_list(["search", "engine", "filter", "ranking", "attribute"])
        .protected_words(["API", "JSON", "HTTP"])
        .multi_way_synonyms(&["search", "query", "lookup"])
        .one_way_synonyms("filter", &["where clause", "predicate"])
        .max_expansions(5)
        .lowercase()
        .build()?;

    let processed = pipeline.process("  SEARCH Filter  ");
    println!("Input:     \"{}\"", processed.original);
    println!("Corrected: \"{}\"", processed.corrected);
    if processed.has_expansions() {
        println!("Expanded:  {}", processed.expanded.to_expanded_string());
    }

    // ======================================================================
    // 3. Loading synonyms from TOML
    // ======================================================================
    println!("\n=== 3. Synonyms from TOML ===\n");

    let toml_content = r#"
        [synonyms]
        multi_way = [
            ["laptop", "notebook", "portable computer"],
            ["phone", "mobile", "cell"],
        ]

        [synonyms.one_way]
        tablet = ["iPad", "Surface"]
        desktop = ["workstation", "tower"]
    "#;

    let mut synonym_map = SynonymMap::new();
    synonym_map
        .load_from_toml_str(toml_content)
        .expect("failed to parse TOML");

    println!(
        "Loaded: {} multi-way groups, {} one-way mappings",
        synonym_map.multi_way_group_count(),
        synonym_map.one_way_mapping_count()
    );

    // Expand some terms
    for term in &["laptop", "phone", "tablet", "desktop"] {
        let expansions = synonym_map.expand_term(term);
        println!("  '{}' -> {:?}", term, expansions);
    }

    // ======================================================================
    // 4. FTS5 and SurrealDB query generation
    // ======================================================================
    println!("\n=== 4. Query generation for databases ===\n");

    let expanded = synonym_map.expand_query("laptop tablet");
    println!("Original query: \"laptop tablet\"");
    println!("FTS5 MATCH:     {}", expanded.to_fts5_match());
    println!(
        "SurrealDB FTS:  {}",
        expanded.to_surrealdb_fts("content", 0)
    );
    println!("Expanded:       {}", expanded.to_expanded_string());

    // ======================================================================
    // 5. Passthrough pipeline (no processing)
    // ======================================================================
    println!("\n=== 5. Passthrough pipeline ===\n");

    let passthrough = QueryPipeline::passthrough();
    let processed = passthrough.process("hello world");
    println!("Input:     \"{}\"", processed.original);
    println!("Corrected: \"{}\"", processed.corrected);
    println!(
        "Has corrections: {}, Has expansions: {}",
        processed.has_corrections(),
        processed.has_expansions()
    );

    println!("\nDone.");
    Ok(())
}
