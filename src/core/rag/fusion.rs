//! Reciprocal Rank Fusion (RRF) and other fusion algorithms.
//!
//! This module provides algorithms for combining multiple ranked result lists
//! into a single unified ranking. RRF is particularly useful for hybrid search
//! where we need to merge keyword and semantic search results.
//!
//! # Reciprocal Rank Fusion
//!
//! RRF combines rankings using the formula:
//!
//! ```text
//! RRF_score(d) = sum over all rankings r: 1 / (k + rank_r(d))
//! ```
//!
//! Where `k` is a constant (typically 60) that mitigates the impact of high
//! rankings from individual lists.

use std::collections::HashMap;
use std::hash::Hash;

use crate::core::rag::types::{RetrievalResult, RetrievalSource};

/// Default RRF constant (k parameter).
pub const DEFAULT_RRF_K: usize = 60;

/// Combine multiple ranked lists using Reciprocal Rank Fusion.
///
/// # Algorithm
///
/// For each document `d` that appears in any list, compute:
/// ```text
/// RRF_score(d) = sum over all rankings r where d appears: 1 / (k + rank_r(d))
/// ```
///
/// Documents are then sorted by their RRF scores in descending order.
///
/// # Precision Note
///
/// Scores are computed as `f32`. For typical result sets (< 10,000 items per list),
/// f32 provides sufficient precision — the smallest RRF contribution at rank 10,000
/// with k=60 is `1/10060 ≈ 9.94e-5`, well within f32's ~7-digit precision.
/// For very deep rankings (> 100k items), f32 precision loss could cause rank
/// instability in the tail, but this is unlikely in practice.
///
/// # Arguments
///
/// * `ranked_lists` - Multiple ranked lists of items to fuse
/// * `k` - The RRF constant (typically 60). Higher values reduce the impact of high rankings.
/// * `limit` - Maximum number of results to return
/// * `id_fn` - Function to extract a unique identifier from each item for deduplication
///
/// # Returns
///
/// A vector of (item, rrf_score) tuples, sorted by score descending.
///
/// # Example
///
/// ```
/// use wilysearch::core::rag::fusion::reciprocal_rank_fusion;
///
/// let keyword_results = vec!["doc1", "doc2", "doc3"];
/// let semantic_results = vec!["doc2", "doc4", "doc1"];
///
/// let fused = reciprocal_rank_fusion(
///     &[keyword_results, semantic_results],
///     60,
///     5,
///     |doc| *doc,  // use the doc itself as ID
/// );
///
/// // doc2 appears at rank 2 in keyword and rank 1 in semantic
/// // doc1 appears at rank 1 in keyword and rank 3 in semantic
/// // doc2 should likely rank higher due to better semantic ranking
/// ```
pub fn reciprocal_rank_fusion<T, F, Id>(
    ranked_lists: &[Vec<T>],
    k: usize,
    limit: usize,
    id_fn: F,
) -> Vec<(T, f32)>
where
    T: Clone,
    F: Fn(&T) -> Id,
    Id: Hash + Eq,
{
    if ranked_lists.is_empty() {
        return Vec::new();
    }

    // Calculate RRF scores for each unique item
    let mut scores: HashMap<Id, (T, f32)> = HashMap::new();

    for list in ranked_lists {
        for (rank, item) in list.iter().enumerate() {
            let id = id_fn(item);
            let rrf_contribution = 1.0 / (k + rank + 1) as f32; // rank is 0-indexed, RRF uses 1-indexed

            scores
                .entry(id)
                .and_modify(|(_, score)| *score += rrf_contribution)
                .or_insert_with(|| (item.clone(), rrf_contribution));
        }
    }

    // Sort by RRF score descending
    let mut results: Vec<_> = scores.into_values().collect();
    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Limit results
    results.truncate(limit);
    results
}

/// Fuse retrieval results from multiple sources using RRF.
///
/// This is a specialized version of RRF for `RetrievalResult` types that
/// preserves document metadata and updates the source to `Hybrid`.
///
/// # Arguments
///
/// * `result_lists` - Multiple lists of retrieval results to fuse
/// * `k` - The RRF constant
/// * `limit` - Maximum number of results to return
/// * `id_fn` - Function to extract document ID for deduplication
///
/// # Returns
///
/// Fused retrieval results with updated scores and `RetrievalSource::Hybrid`.
pub fn fuse_retrieval_results<D, F, Id>(
    result_lists: Vec<Vec<RetrievalResult<D>>>,
    k: usize,
    limit: usize,
    id_fn: F,
) -> Vec<RetrievalResult<D>>
where
    D: Clone,
    F: Fn(&D) -> Id,
    Id: Hash + Eq,
{
    if result_lists.is_empty() {
        return Vec::new();
    }

    // Track scores and keep the best scoring version of each document
    let mut doc_data: HashMap<Id, (RetrievalResult<D>, f32)> = HashMap::new();

    for list in &result_lists {
        for (rank, result) in list.iter().enumerate() {
            let id = id_fn(&result.document);
            let rrf_contribution = 1.0 / (k + rank + 1) as f32;

            doc_data
                .entry(id)
                .and_modify(|(_, score)| *score += rrf_contribution)
                .or_insert_with(|| (result.clone(), rrf_contribution));
        }
    }

    // Sort by RRF score and build final results
    let mut results: Vec<_> = doc_data.into_values().collect();
    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    results
        .into_iter()
        .take(limit)
        .enumerate()
        .map(|(rank, (mut result, rrf_score))| {
            result.score = rrf_score;
            result.source = RetrievalSource::Hybrid;
            result.rank = Some(rank);
            result
        })
        .collect()
}

/// Combine results using a simple weighted score fusion.
///
/// Instead of RRF, this combines results by normalizing and weighting
/// the original scores from each source.
///
/// Documents appearing in only one source receive a score of `0.0` from the
/// missing source. This is by design: a document that only appears in keyword
/// results gets `kw_score * keyword_weight + 0.0 * semantic_weight`, which
/// naturally down-ranks it relative to documents found by both sources.
///
/// # Arguments
///
/// * `keyword_results` - Results from keyword search
/// * `semantic_results` - Results from semantic search
/// * `semantic_ratio` - Weight for semantic results (0.0 to 1.0)
/// * `limit` - Maximum number of results to return
/// * `id_fn` - Function to extract document ID for deduplication
pub fn weighted_score_fusion<D, F, Id>(
    keyword_results: Vec<RetrievalResult<D>>,
    semantic_results: Vec<RetrievalResult<D>>,
    semantic_ratio: f32,
    limit: usize,
    id_fn: F,
) -> Vec<RetrievalResult<D>>
where
    D: Clone,
    F: Fn(&D) -> Id,
    Id: Hash + Eq + Clone,
{
    let keyword_weight = 1.0 - semantic_ratio;
    let semantic_weight = semantic_ratio;

    // Normalize scores within each list
    let keyword_normalized = normalize_scores(keyword_results);
    let semantic_normalized = normalize_scores(semantic_results);

    // Build a map of ID -> (best_doc, keyword_score, semantic_score)
    let mut combined: HashMap<Id, (RetrievalResult<D>, Option<f32>, Option<f32>)> = HashMap::new();

    for result in keyword_normalized {
        let id = id_fn(&result.document);
        combined
            .entry(id)
            .and_modify(|(_, kw, _)| *kw = Some(result.score))
            .or_insert((result.clone(), Some(result.score), None));
    }

    for result in semantic_normalized {
        let id = id_fn(&result.document);
        combined
            .entry(id)
            .and_modify(|(_, _, sem)| *sem = Some(result.score))
            .or_insert((result.clone(), None, Some(result.score)));
    }

    // Calculate combined scores and build final results
    let mut results: Vec<_> = combined
        .into_values()
        .map(|(mut result, kw_score, sem_score)| {
            let kw = kw_score.unwrap_or(0.0);
            let sem = sem_score.unwrap_or(0.0);
            let combined_score = kw * keyword_weight + sem * semantic_weight;

            result.score = combined_score;
            result.source = RetrievalSource::Hybrid;
            result
        })
        .collect();

    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(limit);

    for (rank, result) in results.iter_mut().enumerate() {
        result.rank = Some(rank);
    }

    results
}

/// Normalize scores in a result list to the range [0, 1].
///
/// Uses min-max normalization: `(score - min) / (max - min)`. This correctly
/// handles negative input scores — they are mapped linearly to [0, 1] just
/// like positive scores. For example, scores [-2, 0, 2] become [0, 0.5, 1].
///
/// When all scores are identical (range = 0), all results are normalized to 1.0.
fn normalize_scores<D>(mut results: Vec<RetrievalResult<D>>) -> Vec<RetrievalResult<D>> {
    if results.is_empty() {
        return results;
    }

    let min_score = results
        .iter()
        .map(|r| r.score)
        .fold(f32::INFINITY, f32::min);
    let max_score = results
        .iter()
        .map(|r| r.score)
        .fold(f32::NEG_INFINITY, f32::max);

    let range = max_score - min_score;
    if range > 0.0 {
        for result in &mut results {
            result.score = (result.score - min_score) / range;
        }
    } else {
        // All scores are the same, normalize to 1.0
        for result in &mut results {
            result.score = 1.0;
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rrf_basic() {
        let list1 = vec!["a", "b", "c"];
        let list2 = vec!["b", "c", "d"];

        let fused = reciprocal_rank_fusion(&[list1, list2], 60, 10, |&x| x);

        // b appears at rank 2 in list1 and rank 1 in list2
        // a appears at rank 1 in list1 only
        // b should have highest score

        assert!(!fused.is_empty());
        assert_eq!(fused[0].0, "b"); // b should be first due to better combined ranking
    }

    #[test]
    fn test_rrf_empty_lists() {
        let fused: Vec<(&str, f32)> = reciprocal_rank_fusion(&[], 60, 10, |&x| x);
        assert!(fused.is_empty());
    }

    #[test]
    fn test_rrf_single_list() {
        let list = vec!["a", "b", "c"];
        let fused = reciprocal_rank_fusion(&[list], 60, 10, |&x| x);

        assert_eq!(fused.len(), 3);
        assert_eq!(fused[0].0, "a"); // First item stays first
        assert_eq!(fused[1].0, "b");
        assert_eq!(fused[2].0, "c");
    }

    #[test]
    fn test_rrf_limit() {
        let list1 = vec!["a", "b", "c", "d", "e"];
        let list2 = vec!["f", "g", "h", "i", "j"];

        let fused = reciprocal_rank_fusion(&[list1, list2], 60, 3, |&x| x);
        assert_eq!(fused.len(), 3);
    }

    #[test]
    fn test_fuse_retrieval_results() {
        let kw_results = vec![
            RetrievalResult::new("doc1".to_string(), 0.9, RetrievalSource::Keyword),
            RetrievalResult::new("doc2".to_string(), 0.8, RetrievalSource::Keyword),
        ];
        let sem_results = vec![
            RetrievalResult::new("doc2".to_string(), 0.95, RetrievalSource::Semantic),
            RetrievalResult::new("doc3".to_string(), 0.7, RetrievalSource::Semantic),
        ];

        let fused = fuse_retrieval_results(vec![kw_results, sem_results], 60, 10, |d| d.clone());

        assert_eq!(fused.len(), 3);
        // doc2 should be first as it appears in both lists
        assert_eq!(fused[0].document, "doc2");
        assert_eq!(fused[0].source, RetrievalSource::Hybrid);
    }

    #[test]
    fn test_weighted_score_fusion() {
        let kw_results = vec![
            RetrievalResult::new("doc1".to_string(), 1.0, RetrievalSource::Keyword),
            RetrievalResult::new("doc2".to_string(), 0.5, RetrievalSource::Keyword),
        ];
        let sem_results = vec![
            RetrievalResult::new("doc2".to_string(), 1.0, RetrievalSource::Semantic),
            RetrievalResult::new("doc3".to_string(), 0.8, RetrievalSource::Semantic),
        ];

        // 70% semantic weight
        let fused = weighted_score_fusion(kw_results, sem_results, 0.7, 10, |d| d.clone());

        assert_eq!(fused.len(), 3);
        // All results should be marked as Hybrid
        for result in &fused {
            assert_eq!(result.source, RetrievalSource::Hybrid);
        }
    }

    #[test]
    fn test_normalize_scores() {
        let results = vec![
            RetrievalResult::new("a", 10.0, RetrievalSource::Keyword),
            RetrievalResult::new("b", 5.0, RetrievalSource::Keyword),
            RetrievalResult::new("c", 0.0, RetrievalSource::Keyword),
        ];

        let normalized = normalize_scores(results);

        assert_eq!(normalized[0].score, 1.0); // max -> 1.0
        assert_eq!(normalized[1].score, 0.5); // middle -> 0.5
        assert_eq!(normalized[2].score, 0.0); // min -> 0.0
    }
}
