mod benchmark;
pub mod llama;
mod memory;
mod mock;
mod rag;
mod reasoning;
mod router;
mod scheduler;
mod sink;
mod tools;
mod traits;

pub use benchmark::{benchmark_after_load, BenchmarkStore};
pub use llama::LlamaCppEngine;
pub use memory::{Admission, EvictReason, MemoryGuard, ThermalState};
pub use mock::MockEngine;
pub use rag::{
    bm25_rank, chunk_document, cosine_rank, evaluate_lexical_hit_at_5, evaluate_lexical_hit_at_k,
    format_citation, hybrid_search, Chunk, ChunkHit, ChunkRecord, ChunkerConfig, EvalQuestion,
    EvalResult, PageText,
};
pub use reasoning::{no_think_strategy_for_family, NoThinkStrategy, ReasoningSplitter, Split};
pub use router::{route, EnginePrefs, EngineRegistry};
pub use scheduler::{sse_queued_comment, Priority, Scheduler};
pub use sink::BatchingSink;
pub use tools::{
    parse_tool_calls_hermes, parse_tool_calls_llama3, run_tool_loop, ParsedToolCall, ToolRegistry,
};
pub use traits::{Engine, TokenSink};
