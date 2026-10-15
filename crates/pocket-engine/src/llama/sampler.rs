//! Sampler chain construction for llama.cpp text generation.
//!
//! Chain order per ENG-05:
//! penalties (repetition penalty) -> top_k -> top_p -> min_p -> temperature -> distribution.

use std::time::SystemTime;

use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::sampling::LlamaSampler;
use pocket_types::{PocketError, SamplingParams};

/// Constructs the full sampler chain with a specific vocabulary size.
pub fn build_sampler_chain_with_vocab(
    n_vocab: i32,
    params: &SamplingParams,
) -> Result<LlamaSampler, PocketError> {
    let seed = params.seed.map(|s| s as u64 as u32).unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u32)
            .unwrap_or(42)
    });

    // 1. Penalties (repetition penalty from params).
    // Penalize the last 64 tokens. Frequency and presence penalties are 0.0 (disabled).
    let rep_penalty = if params.repetition_penalty > 0.0 {
        params.repetition_penalty
    } else {
        1.0
    };
    let penalty_sampler = LlamaSampler::penalties(n_vocab, 64, rep_penalty, 0.0, 0.0);

    // 2. top_k
    let top_k_sampler = LlamaSampler::top_k(params.top_k);

    // 3. top_p
    let top_p_sampler = LlamaSampler::top_p(params.top_p, 1);

    // 4. min_p
    let min_p_sampler = LlamaSampler::min_p(params.min_p, 1);

    // 5. temperature
    let temp_sampler = LlamaSampler::temp(params.temperature);

    // 6. distribution-based final sample seeded by params.seed (if Some, else random)
    let dist_sampler = LlamaSampler::dist(seed);

    let chain = LlamaSampler::chain_simple([
        penalty_sampler,
        top_k_sampler,
        top_p_sampler,
        min_p_sampler,
        temp_sampler,
        dist_sampler,
    ]);

    Ok(chain)
}

/// Constructs the full sampler chain from a loaded `LlamaModel` and `SamplingParams`.
pub fn build_sampler_chain(
    model: &LlamaModel,
    params: &SamplingParams,
) -> Result<LlamaSampler, PocketError> {
    let n_vocab = model.vocab().n_tokens();
    build_sampler_chain_with_vocab(n_vocab, params)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_sampler_chain_successfully() {
        let params = SamplingParams {
            temperature: 0.8,
            top_p: 0.9,
            top_k: 40,
            min_p: 0.05,
            max_tokens: 128,
            stop: vec![],
            seed: Some(1337),
            repetition_penalty: 1.15,
        };

        let sampler = build_sampler_chain_with_vocab(32000, &params);
        assert!(sampler.is_ok());
    }

    #[test]
    fn sampler_honors_explicit_seed() {
        let params = SamplingParams {
            seed: Some(9999),
            ..Default::default()
        };
        let sampler = build_sampler_chain_with_vocab(32000, &params).unwrap();
        // LlamaSampler::get_seed returns the first non-default seed in the chain
        assert_eq!(sampler.get_seed(), 9999);
    }
}
