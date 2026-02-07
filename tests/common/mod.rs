use wilysearch::core::MeilisearchOptions;
use wilysearch::engine::Engine;
use wilysearch::traits::*;
use wilysearch::types::*;
use serde_json::{json, Value};
use tempfile::TempDir;

/// Test context that creates a temporary Engine instance.
///
/// The LMDB environment and all index data are stored in a temporary directory
/// that is automatically cleaned up when the `TestContext` is dropped.
pub struct TestContext {
    pub engine: Engine,
    _temp_dir: TempDir,
}

impl TestContext {
    /// Create a new test context with a fresh Engine instance.
    pub fn new() -> Self {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let options = MeilisearchOptions {
            db_path: temp_dir.path().to_path_buf(),
            max_index_size: 100 * 1024 * 1024, // 100 MB for tests
            max_task_db_size: 10 * 1024 * 1024, // 10 MB for tests
        };
        let engine = Engine::new(options).expect("failed to create Engine");
        Self {
            engine,
            _temp_dir: temp_dir,
        }
    }
}

/// Create a test index with sample movies via the trait API.
///
/// Creates the index, adds 10 sample movie documents, and configures
/// the primary key to "id".
pub fn create_test_index(ctx: &TestContext, uid: &str) {
    ctx.engine
        .create_index(&CreateIndexRequest {
            uid: uid.to_string(),
            primary_key: Some("id".to_string()),
        })
        .expect("failed to create index");

    ctx.engine
        .add_or_replace_documents(uid, &sample_movies(), &AddDocumentsQuery::default())
        .expect("failed to add documents");
}

#[allow(dead_code)]
/// Create a test index pre-configured with filterable/sortable attributes.
pub fn create_configured_index(ctx: &TestContext, uid: &str) {
    create_test_index(ctx, uid);

    let settings = Settings {
        filterable_attributes: Some(vec![
            "year".to_string(),
            "genres".to_string(),
            "rating".to_string(),
        ]),
        sortable_attributes: Some(vec![
            "year".to_string(),
            "rating".to_string(),
        ]),
        ..Default::default()
    };

    ctx.engine
        .update_settings(uid, &settings)
        .expect("failed to update settings");
}

/// Returns 10 sample movie documents.
pub fn sample_movies() -> Vec<Value> {
    vec![
        json!({
            "id": 1,
            "title": "The Dark Knight",
            "genres": ["Action", "Crime", "Drama"],
            "year": 2008,
            "rating": 9.0
        }),
        json!({
            "id": 2,
            "title": "Inception",
            "genres": ["Action", "Adventure", "Sci-Fi"],
            "year": 2010,
            "rating": 8.8
        }),
        json!({
            "id": 3,
            "title": "The Shawshank Redemption",
            "genres": ["Drama"],
            "year": 1994,
            "rating": 9.3
        }),
        json!({
            "id": 4,
            "title": "Pulp Fiction",
            "genres": ["Crime", "Drama"],
            "year": 1994,
            "rating": 8.9
        }),
        json!({
            "id": 5,
            "title": "The Godfather",
            "genres": ["Crime", "Drama"],
            "year": 1972,
            "rating": 9.2
        }),
        json!({
            "id": 6,
            "title": "Forrest Gump",
            "genres": ["Drama", "Romance"],
            "year": 1994,
            "rating": 8.8
        }),
        json!({
            "id": 7,
            "title": "Interstellar",
            "genres": ["Adventure", "Drama", "Sci-Fi"],
            "year": 2014,
            "rating": 8.7
        }),
        json!({
            "id": 8,
            "title": "The Matrix",
            "genres": ["Action", "Sci-Fi"],
            "year": 1999,
            "rating": 8.7
        }),
        json!({
            "id": 9,
            "title": "Fight Club",
            "genres": ["Drama"],
            "year": 1999,
            "rating": 8.8
        }),
        json!({
            "id": 10,
            "title": "The Lord of the Rings",
            "genres": ["Action", "Adventure", "Drama"],
            "year": 2001,
            "rating": 8.9
        }),
    ]
}

#[allow(dead_code)]
/// Returns 5 sample book documents.
pub fn sample_books() -> Vec<Value> {
    vec![
        json!({
            "id": 1,
            "title": "To Kill a Mockingbird",
            "author": "Harper Lee",
            "year": 1960,
            "pages": 281
        }),
        json!({
            "id": 2,
            "title": "1984",
            "author": "George Orwell",
            "year": 1949,
            "pages": 328
        }),
        json!({
            "id": 3,
            "title": "The Great Gatsby",
            "author": "F. Scott Fitzgerald",
            "year": 1925,
            "pages": 180
        }),
        json!({
            "id": 4,
            "title": "One Hundred Years of Solitude",
            "author": "Gabriel Garcia Marquez",
            "year": 1967,
            "pages": 417
        }),
        json!({
            "id": 5,
            "title": "Brave New World",
            "author": "Aldous Huxley",
            "year": 1932,
            "pages": 311
        }),
    ]
}
