//! Hybrid Search Example
//!
//! Demonstrates how to configure and use hybrid search, which combines
//! traditional keyword search with vector/semantic search:
//!
//! 1. Using the `HybridQuery` field on a `SearchQuery` to request hybrid behavior
//! 2. Using the legacy `HybridSearchQuery` for explicit vector + keyword fusion
//! 3. Controlling the `semantic_ratio` to balance keyword vs semantic results
//!
//! **Note:** Full hybrid search requires a configured embedder (OpenAI, Ollama,
//! HuggingFace, or user-provided vectors). This example shows the API surface
//! and falls back to keyword-only search when no embedder is configured.

use wilysearch::core::{
    HybridQuery, HybridSearchQuery, Meilisearch, MeilisearchOptions, SearchQuery, Settings,
};
use serde_json::json;

fn main() -> wilysearch::core::Result<()> {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let options = MeilisearchOptions {
        db_path: tmp_dir.path().to_path_buf(),
        ..Default::default()
    };
    let meili = Meilisearch::new(options)?;

    let index = meili.create_index("articles", Some("id"))?;
    println!("Index 'articles' created.");

    // -- Add some documents --
    let articles = vec![
        json!({
            "id": 1,
            "title": "Introduction to Machine Learning",
            "content": "Machine learning is a subset of artificial intelligence that enables systems to learn from data.",
            "category": "AI"
        }),
        json!({
            "id": 2,
            "title": "Deep Learning with Neural Networks",
            "content": "Neural networks are computing systems inspired by biological neural networks in the brain.",
            "category": "AI"
        }),
        json!({
            "id": 3,
            "title": "Natural Language Processing Fundamentals",
            "content": "NLP is a field of AI focused on the interaction between computers and human language.",
            "category": "NLP"
        }),
        json!({
            "id": 4,
            "title": "Cooking with Seasonal Vegetables",
            "content": "Fresh seasonal vegetables bring out the best flavors in home cooking.",
            "category": "Food"
        }),
    ];
    index.add_documents(articles, None)?;
    println!("4 articles added.\n");

    // ======================================================================
    // Approach 1: SearchQuery with HybridQuery field
    // ======================================================================
    // This is the modern API for requesting hybrid search. The `HybridQuery`
    // specifies which embedder to use and the semantic_ratio balance.
    //
    // Without a configured embedder, this will fall back to keyword search.

    println!("=== Approach 1: SearchQuery with HybridQuery ===");
    let query = SearchQuery::new("artificial intelligence")
        .with_hybrid(HybridQuery::new("default").with_semantic_ratio(0.5))
        .with_ranking_score(true)
        .with_limit(5);

    let result = index.search(&query)?;
    println!(
        "Query: 'artificial intelligence' (hybrid, semantic_ratio=0.5)"
    );
    println!(
        "Hits: {}, Time: {}ms",
        result.hits.len(),
        result.processing_time_ms
    );
    for hit in &result.hits {
        println!(
            "  - {} (score: {:.4})",
            hit.document["title"],
            hit.ranking_score.unwrap_or(0.0)
        );
    }

    // ======================================================================
    // Approach 2: SearchQuery with explicit vector
    // ======================================================================
    // You can also provide a pre-computed embedding vector directly.
    // This is useful when you handle embedding generation externally.

    println!("\n=== Approach 2: SearchQuery with explicit vector ===");
    // In practice, this vector would come from an embedding model.
    // Here we use a placeholder to illustrate the API.
    let fake_embedding = vec![0.1, 0.2, 0.3, 0.4, 0.5];
    let query = SearchQuery::new("neural networks")
        .with_vector(fake_embedding)
        .with_hybrid(HybridQuery::new("default").with_semantic_ratio(0.7))
        .with_ranking_score(true);

    let result = index.search(&query)?;
    println!("Query: 'neural networks' (with explicit vector, semantic_ratio=0.7)");
    println!("Hits: {}", result.hits.len());
    for hit in &result.hits {
        println!(
            "  - {} (score: {:.4})",
            hit.document["title"],
            hit.ranking_score.unwrap_or(0.0)
        );
    }

    // ======================================================================
    // Approach 3: Legacy HybridSearchQuery
    // ======================================================================
    // The legacy API provides a separate struct that wraps SearchQuery with
    // explicit vector and semantic_ratio fields. It delegates to
    // Index::hybrid_search() which merges keyword and vector results.

    println!("\n=== Approach 3: Legacy HybridSearchQuery ===");
    let hybrid_query = HybridSearchQuery::new("learning systems")
        .with_semantic_ratio(0.3) // Favor keyword search
        .with_ranking_score(true)
        .with_limit(5);

    // Note: Without a VectorStore configured, this falls back to keyword search.
    let hybrid_result = index.hybrid_search(&hybrid_query)?;
    println!(
        "Query: 'learning systems' (semantic_ratio=0.3, keyword-heavy)"
    );
    println!(
        "Hits: {}, Semantic hits: {:?}",
        hybrid_result.result.hits.len(),
        hybrid_result.semantic_hit_count
    );
    for hit in &hybrid_result.result.hits {
        println!(
            "  - {} (score: {:.4})",
            hit.document["title"],
            hit.ranking_score.unwrap_or(0.0)
        );
    }

    // ======================================================================
    // Configuring an embedder (for reference)
    // ======================================================================
    // To enable true hybrid search, configure an embedder in settings:
    //
    //   use wilysearch::core::EmbedderSettings;
    //
    //   let settings = Settings::new()
    //       .with_embedder("default", EmbedderSettings::openai("sk-your-api-key"))
    //       // Or Ollama:
    //       // .with_embedder("default", EmbedderSettings::ollama(
    //       //     "http://localhost:11434",
    //       //     "nomic-embed-text",
    //       // ))
    //       // Or user-provided vectors:
    //       // .with_embedder("default", EmbedderSettings::user_provided(384))
    //       ;
    //   index.update_settings(&settings)?;
    //
    // After configuring an embedder, the hybrid search methods will produce
    // genuine semantic results blended with keyword results.

    // Suppress unused import warning -- Settings is shown in the doc comment above.
    let _ = Settings::new();

    println!("\nDone.");
    Ok(())
}
