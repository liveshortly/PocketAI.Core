//! ENG-11's hybrid search: BM25 (lexical) and cosine similarity (semantic)
//! ranked lists combined by reciprocal rank fusion. Brute-force cosine is
//! fine up to ~50k chunks per the card; this isn't backed by FTS5/SQLite
//! yet (D45) - it operates on an in-memory `&[ChunkRecord]` corpus.
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct ChunkRecord {
    pub document: String,
    pub page: u32,
    pub text: String,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChunkHit {
    pub document: String,
    pub page: u32,
    pub text: String,
    pub score: f32,
}

const BM25_K1: f32 = 1.2;
const BM25_B: f32 = 0.75;

fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).map(|t| t.to_lowercase()).collect()
}

/// Ranks `corpus` against `query` by BM25 score, descending, keeping only
/// chunks that match at least one query term.
pub fn bm25_rank(query: &str, corpus: &[ChunkRecord]) -> Vec<(usize, f32)> {
    let query_terms = tokenize(query);
    if query_terms.is_empty() || corpus.is_empty() {
        return Vec::new();
    }

    let docs: Vec<Vec<String>> = corpus.iter().map(|c| tokenize(&c.text)).collect();
    let avg_len = docs.iter().map(|d| d.len()).sum::<usize>() as f32 / docs.len() as f32;
    let n = docs.len() as f32;

    let mut doc_freq: HashMap<&str, usize> = HashMap::new();
    for term in &query_terms {
        let df = docs.iter().filter(|d| d.iter().any(|t| t == term)).count();
        doc_freq.insert(term.as_str(), df);
    }

    let mut scores: Vec<(usize, f32)> = docs
        .iter()
        .enumerate()
        .map(|(i, doc)| {
            let doc_len = doc.len() as f32;
            let score = query_terms
                .iter()
                .map(|term| {
                    let tf = doc.iter().filter(|t| *t == term).count() as f32;
                    if tf == 0.0 {
                        return 0.0;
                    }
                    let df = *doc_freq.get(term.as_str()).unwrap_or(&0) as f32;
                    let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
                    idf * (tf * (BM25_K1 + 1.0)) / (tf + BM25_K1 * (1.0 - BM25_B + BM25_B * doc_len / avg_len))
                })
                .sum::<f32>();
            (i, score)
        })
        .filter(|(_, score)| *score > 0.0)
        .collect();

    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    scores
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

/// Brute-force cosine ranking, descending.
pub fn cosine_rank(query_embedding: &[f32], corpus: &[ChunkRecord]) -> Vec<(usize, f32)> {
    let mut scores: Vec<(usize, f32)> = corpus
        .iter()
        .enumerate()
        .map(|(i, c)| (i, cosine_similarity(query_embedding, &c.embedding)))
        .collect();
    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    scores
}

/// Reciprocal rank fusion: each ranked list contributes `1/(k + rank)`
/// (1-based rank) per index; totals summed across lists, sorted descending.
pub fn reciprocal_rank_fusion(rank_lists: &[Vec<usize>], k: f32) -> Vec<(usize, f32)> {
    let mut totals: HashMap<usize, f32> = HashMap::new();
    for list in rank_lists {
        for (rank, &idx) in list.iter().enumerate() {
            *totals.entry(idx).or_insert(0.0) += 1.0 / (k + (rank + 1) as f32);
        }
    }
    let mut fused: Vec<(usize, f32)> = totals.into_iter().collect();
    fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(&b.0)));
    fused
}

/// BM25 top 50 + cosine top 50 -> RRF(k=60) -> top `top_k`, per the card.
pub fn hybrid_search(query: &str, query_embedding: &[f32], corpus: &[ChunkRecord], top_k: usize) -> Vec<ChunkHit> {
    let bm25_top: Vec<usize> = bm25_rank(query, corpus).into_iter().take(50).map(|(i, _)| i).collect();
    let cosine_top: Vec<usize> = cosine_rank(query_embedding, corpus).into_iter().take(50).map(|(i, _)| i).collect();
    let fused = reciprocal_rank_fusion(&[bm25_top, cosine_top], 60.0);

    fused
        .into_iter()
        .take(top_k)
        .map(|(i, score)| {
            let c = &corpus[i];
            ChunkHit { document: c.document.clone(), page: c.page, text: c.text.clone(), score }
        })
        .collect()
}

pub fn format_citation(index: usize, document: &str, page: u32) -> String {
    format!("[{index}] {document} p.{page}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(document: &str, page: u32, text: &str, embedding: Vec<f32>) -> ChunkRecord {
        ChunkRecord { document: document.into(), page, text: text.into(), embedding }
    }

    #[test]
    fn bm25_ranks_the_chunk_containing_the_query_terms_first() {
        let corpus = vec![
            record("doc", 1, "the weather today is sunny and warm", vec![]),
            record("doc", 2, "quantum computing uses qubits for superposition", vec![]),
        ];
        let ranked = bm25_rank("qubits superposition", &corpus);
        assert_eq!(ranked[0].0, 1);
    }

    #[test]
    fn bm25_returns_empty_for_a_query_with_no_matching_terms() {
        let corpus = vec![record("doc", 1, "the weather today is sunny", vec![])];
        assert!(bm25_rank("blockchain cryptocurrency", &corpus).is_empty());
    }

    #[test]
    fn cosine_ranks_the_closest_embedding_first() {
        let corpus = vec![
            record("doc", 1, "a", vec![1.0, 0.0, 0.0]),
            record("doc", 2, "b", vec![0.0, 1.0, 0.0]),
        ];
        let ranked = cosine_rank(&[0.9, 0.1, 0.0], &corpus);
        assert_eq!(ranked[0].0, 0);
    }

    #[test]
    fn reciprocal_rank_fusion_combines_two_lists_with_the_expected_scores() {
        // index 5 is #1 in both lists -> highest fused score.
        let fused = reciprocal_rank_fusion(&[vec![5, 2, 7], vec![5, 7, 2]], 60.0);
        assert_eq!(fused[0].0, 5);
        let expected_top = 1.0 / 61.0 + 1.0 / 61.0;
        assert!((fused[0].1 - expected_top).abs() < 1e-6);
    }

    #[test]
    fn reciprocal_rank_fusion_includes_an_index_present_in_only_one_list() {
        let fused = reciprocal_rank_fusion(&[vec![1], vec![2]], 60.0);
        assert_eq!(fused.len(), 2);
    }

    #[test]
    fn hybrid_search_surfaces_the_obviously_relevant_chunk_first() {
        let corpus = vec![
            record("manual", 1, "installation requires python 3.10 or later", vec![1.0, 0.0]),
            record("manual", 2, "the warranty covers manufacturing defects for one year", vec![0.0, 1.0]),
        ];
        let hits = hybrid_search("python installation requirements", &[1.0, 0.0], &corpus, 1);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].page, 1);
        assert_eq!(hits[0].document, "manual");
    }

    #[test]
    fn citation_format_matches_the_spec() {
        assert_eq!(format_citation(1, "name", 12), "[1] name p.12");
    }
}
