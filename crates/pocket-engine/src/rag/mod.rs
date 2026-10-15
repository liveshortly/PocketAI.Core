pub mod chunker;
pub mod eval;
pub mod search;

pub use chunker::{chunk_document, Chunk, ChunkerConfig, PageText};
pub use eval::{evaluate_lexical_hit_at_5, evaluate_lexical_hit_at_k, EvalQuestion, EvalResult};
pub use search::{bm25_rank, cosine_rank, format_citation, hybrid_search, ChunkHit, ChunkRecord};
