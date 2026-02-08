mod common;

use std::sync::{Arc, Barrier};
use std::thread;

use common::TestContext;
use serde_json::json;
use wilysearch::traits::*;
use wilysearch::types::*;

#[test]
fn test_engine_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<wilysearch::engine::Engine>();
}

/// Spawn 8 threads, each creating a uniquely-named index. All creations should
/// succeed. After joining, verify that all 8 indexes are visible via list_indexes.
#[test]
fn test_concurrent_index_creation() {
    let ctx = TestContext::new();
    let engine = Arc::new(ctx.engine);

    let thread_count = 8;
    let barrier = Arc::new(Barrier::new(thread_count));

    let handles: Vec<_> = (0..thread_count)
        .map(|i| {
            let engine = Arc::clone(&engine);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let uid = format!("index_{i}");
                barrier.wait();
                engine
                    .create_index(&CreateIndexRequest {
                        uid: uid.clone(),
                        primary_key: Some("id".to_string()),
                    })
                    .unwrap_or_else(|e| panic!("thread {i} failed to create index: {e}"));
                uid
            })
        })
        .collect();

    let created_uids: Vec<String> = handles
        .into_iter()
        .map(|h| h.join().expect("thread panicked"))
        .collect();

    let list = engine
        .list_indexes(&PaginationQuery {
            offset: None,
            limit: Some(20),
        })
        .expect("failed to list indexes");

    assert_eq!(
        list.total, thread_count as u64,
        "expected {thread_count} indexes, got {}",
        list.total
    );

    let listed_uids: Vec<&str> = list.results.iter().map(|idx| idx.uid.as_str()).collect();
    for uid in &created_uids {
        assert!(
            listed_uids.contains(&uid.as_str()),
            "index '{uid}' missing from list_indexes"
        );
    }
}

/// Create an index with initial documents, then concurrently run one writer
/// thread that adds more documents and 4 reader threads that search the index.
/// All operations must complete without panics. Readers may see either the old
/// or updated document set -- the important thing is that no operation errors
/// out or causes undefined behavior.
#[test]
fn test_search_during_write() {
    let ctx = TestContext::new();
    let engine = Arc::new(ctx.engine);

    // Create the index and seed it with initial documents.
    engine
        .create_index(&CreateIndexRequest {
            uid: "movies".to_string(),
            primary_key: Some("id".to_string()),
        })
        .expect("failed to create index");

    engine
        .add_or_replace_documents("movies", &common::sample_movies(), &AddDocumentsQuery::default())
        .expect("failed to seed documents");

    let reader_count = 4;
    let total_threads = 1 + reader_count; // 1 writer + 4 readers
    let barrier = Arc::new(Barrier::new(total_threads));

    // Writer thread: adds a batch of new documents (ids 100..109).
    let writer_engine = Arc::clone(&engine);
    let writer_barrier = Arc::clone(&barrier);
    let writer = thread::spawn(move || {
        let new_docs: Vec<serde_json::Value> = (100..110)
            .map(|i| {
                json!({
                    "id": i,
                    "title": format!("Concurrent Movie {i}"),
                    "genres": ["Test"],
                    "year": 2024,
                    "rating": 7.0
                })
            })
            .collect();

        writer_barrier.wait();
        writer_engine
            .add_or_replace_documents("movies", &new_docs, &AddDocumentsQuery::default())
            .expect("writer failed to add documents");
    });

    // Reader threads: each performs several searches.
    let readers: Vec<_> = (0..reader_count)
        .map(|i| {
            let engine = Arc::clone(&engine);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                for _ in 0..5 {
                    let result = engine
                        .search(
                            "movies",
                            &SearchRequest {
                                q: Some("dark knight".to_string()),
                                limit: Some(20),
                                ..Default::default()
                            },
                        )
                        .unwrap_or_else(|e| panic!("reader {i} search failed: {e}"));

                    // We should always find at least The Dark Knight from the
                    // seed data, regardless of whether the writer has finished.
                    assert!(
                        !result.hits.is_empty(),
                        "reader {i}: expected at least one hit for 'dark knight'"
                    );
                }
            })
        })
        .collect();

    writer.join().expect("writer thread panicked");
    for (i, handle) in readers.into_iter().enumerate() {
        handle
            .join()
            .unwrap_or_else(|_| panic!("reader thread {i} panicked"));
    }

    // After all threads complete, verify the final document count includes both
    // the original 10 and the 10 added by the writer.
    let stats = engine.index_stats("movies").expect("failed to get stats");
    assert_eq!(
        stats.number_of_documents, 20,
        "expected 20 documents after concurrent write, got {}",
        stats.number_of_documents
    );
}

/// Create 4 indexes. Spawn threads that read index info concurrently while
/// another thread deletes the indexes one by one. Reads should either succeed
/// (returning valid IndexInfo) or fail with an index-not-found error -- they
/// must never panic or trigger undefined behavior.
#[test]
fn test_delete_during_read() {
    let ctx = TestContext::new();
    let engine = Arc::new(ctx.engine);

    let index_names: Vec<String> = (0..4).map(|i| format!("idx_{i}")).collect();

    for name in &index_names {
        engine
            .create_index(&CreateIndexRequest {
                uid: name.clone(),
                primary_key: Some("id".to_string()),
            })
            .expect("failed to create index");
    }

    let reader_count = 4;
    let total_threads = 1 + reader_count; // 1 deleter + 4 readers
    let barrier = Arc::new(Barrier::new(total_threads));

    // Deleter thread: removes each index in sequence, retrying if a reader
    // currently holds an Arc<Index> reference (IndexInUse).
    let deleter_engine = Arc::clone(&engine);
    let deleter_barrier = Arc::clone(&barrier);
    let deleter_names = index_names.clone();
    let deleter = thread::spawn(move || {
        deleter_barrier.wait();
        for name in &deleter_names {
            for attempt in 0..50 {
                match deleter_engine.delete_index(name) {
                    Ok(_) => break,
                    Err(_) if attempt < 49 => {
                        thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(e) => panic!("failed to delete '{name}' after 50 attempts: {e}"),
                }
            }
        }
    });

    // Reader threads: repeatedly try to get_index on all 4 names.
    let readers: Vec<_> = (0..reader_count)
        .map(|i| {
            let engine = Arc::clone(&engine);
            let barrier = Arc::clone(&barrier);
            let names = index_names.clone();
            thread::spawn(move || {
                barrier.wait();
                for _ in 0..10 {
                    for name in &names {
                        match engine.get_index(name) {
                            Ok(info) => {
                                // If the read succeeds, the uid must match.
                                assert_eq!(
                                    info.uid, *name,
                                    "reader {i}: uid mismatch, expected '{name}', got '{}'",
                                    info.uid
                                );
                            }
                            Err(_) => {
                                // Index was already deleted -- this is expected.
                            }
                        }
                    }
                }
            })
        })
        .collect();

    deleter.join().expect("deleter thread panicked");
    for (i, handle) in readers.into_iter().enumerate() {
        handle
            .join()
            .unwrap_or_else(|_| panic!("reader thread {i} panicked"));
    }

    // After everything settles, all 4 indexes should be gone.
    for name in &index_names {
        assert!(
            engine.get_index(name).is_err(),
            "index '{name}' should have been deleted"
        );
    }
}
