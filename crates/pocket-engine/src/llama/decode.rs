//! Dedicated OS thread runner for llama.cpp token generation and streaming.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::token::LlamaToken;
use pocket_types::{
    EngineKind, FinishReason, GenerateDelta, GenerateRequest, GenerateStats, PocketError,
};
use tokio::sync::mpsc::Sender;

use super::sampler::build_sampler_chain;

pub enum DecodeMessage {
    Delta(GenerateDelta),
    Done(GenerateStats),
    Error(PocketError),
}

/// Prompt input to the decode thread runner: either pure text tokens or multimodal chunks.
pub enum PromptInput {
    TextTokens(Vec<LlamaToken>),
    Multimodal {
        chunks: llama_cpp_2::mtmd::MtmdInputChunks,
        mtmd_arc: Arc<Mutex<llama_cpp_2::mtmd::MtmdContext>>,
    },
}

/// Runs the blocking llama.cpp decode loop on a dedicated OS thread.
pub fn run_decode_thread(
    // Drop-order safety for the `LlamaContext<'static>` transmute in `load`:
    // the context must be freed before the model it borrows. Parameters drop
    // in reverse declaration order, so `model_arc` is deliberately declared
    // before `context_arc` — the context (and its guard) drop first on every
    // exit path, even when unload raced a decode and these are the last Arcs.
    model_arc: Arc<LlamaModel>,
    context_arc: Arc<Mutex<LlamaContext<'static>>>,
    prompt_input: PromptInput,
    req: GenerateRequest,
    cancel_flag: Arc<AtomicBool>,
    n_ctx: u32,
    tx: Sender<DecodeMessage>,
) {
    let mut context = match context_arc.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    // Reset KV cache before beginning new generation
    context.clear_kv_cache();

    let n_batch = 512.min(n_ctx as usize);
    let mut batch = LlamaBatch::new(n_batch, 1);
    let is_multimodal = matches!(prompt_input, PromptInput::Multimodal { .. });

    // ttft includes prompt evaluation (prefill), not just the first decode step.
    let decode_start = Instant::now();

    let (total_prompt, mut current_pos) = match prompt_input {
        PromptInput::TextTokens(prompt_tokens) => {
            let total = prompt_tokens.len();
            // 1. Evaluate prompt tokens into KV cache in chunks of up to n_batch
            for (chunk_idx, chunk) in prompt_tokens.chunks(n_batch).enumerate() {
                if cancel_flag.load(Ordering::SeqCst) {
                    let stats = GenerateStats {
                        prompt_tokens: total as u32,
                        completion_tokens: 0,
                        ttft_ms: 0,
                        decode_tps: 0.0,
                        finish: FinishReason::Cancelled,
                    };
                    let _ = tx.blocking_send(DecodeMessage::Done(stats));
                    return;
                }

                batch.clear();
                let base_pos = chunk_idx * n_batch;
                for (i, &token) in chunk.iter().enumerate() {
                    let pos = (base_pos + i) as i32;
                    let is_last = (base_pos + i) == (total - 1);
                    if let Err(e) = batch.add(token, pos, &[0], is_last) {
                        let _ = tx.blocking_send(DecodeMessage::Error(PocketError::EngineFailed {
                            engine: EngineKind::LlamaCpp,
                            message: format!("failed adding prompt token to batch: {e}"),
                        }));
                        return;
                    }
                }

                if let Err(e) = context.decode(&mut batch) {
                    let _ = tx.blocking_send(DecodeMessage::Error(PocketError::EngineFailed {
                        engine: EngineKind::LlamaCpp,
                        message: format!("failed decoding prompt batch: {e}"),
                    }));
                    return;
                }
            }
            (total, total as i32)
        }
        PromptInput::Multimodal { chunks, mtmd_arc } => {
            let total = chunks.total_tokens();
            if cancel_flag.load(Ordering::SeqCst) {
                let stats = GenerateStats {
                    prompt_tokens: total as u32,
                    completion_tokens: 0,
                    ttft_ms: 0,
                    decode_tps: 0.0,
                    finish: FinishReason::Cancelled,
                };
                let _ = tx.blocking_send(DecodeMessage::Done(stats));
                return;
            }

            let mut mtmd_ctx = match mtmd_arc.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };

            let new_n_past =
                match chunks.eval_chunks(&mut mtmd_ctx, &mut context, 0, 0, n_batch as i32, true) {
                    Ok(pos) => pos,
                    Err(e) => {
                        let _ = tx.blocking_send(DecodeMessage::Error(PocketError::EngineFailed {
                            engine: EngineKind::LlamaCpp,
                            message: format!("failed decoding multimodal chunks: {e}"),
                        }));
                        return;
                    }
                };
            (total, new_n_past)
        }
    };

    // 2. Initialize sampler chain
    let mut sampler = match build_sampler_chain(&model_arc, &req.params) {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.blocking_send(DecodeMessage::Error(e));
            return;
        }
    };

    // 3. Autoregressive token generation loop
    let mut first_token_time: Option<Instant> = None;
    let mut ttft_ms = 0u32;
    let mut completion_tokens = 0u32;
    let max_tokens = req.params.max_tokens;

    let mut batch_text = String::new();
    let mut batch_tokens = 0u32;
    let mut last_emit = Instant::now();
    let mut byte_accumulator: Vec<u8> = Vec::new();
    let mut generated_text = String::new();
    let mut finish_reason = FinishReason::Stop;

    while completion_tokens < max_tokens && (current_pos as u32) < n_ctx {
        if cancel_flag.load(Ordering::SeqCst) {
            finish_reason = FinishReason::Cancelled;
            break;
        }

        // Sample next token from the last evaluated position
        let sample_idx = if completion_tokens == 0 && is_multimodal {
            -1
        } else {
            batch.n_tokens() - 1
        };
        let token = sampler.sample(&context, sample_idx);
        sampler.accept(token);

        if first_token_time.is_none() {
            let now = Instant::now();
            ttft_ms = now.duration_since(decode_start).as_millis() as u32;
            first_token_time = Some(now);
        }
        completion_tokens += 1;

        // Check End-Of-Generation (EOG) token
        if model_arc.vocab().is_eog(token) {
            finish_reason = FinishReason::Stop;
            break;
        }

        // Detokenize piece
        let piece_bytes = model_arc.vocab().token_to_piece(token, false, None);
        byte_accumulator.extend_from_slice(&piece_bytes);

        // Decode valid UTF-8 prefix, leaving incomplete multibyte fragments in accumulator
        let mut emitted_piece = String::new();
        match std::str::from_utf8(&byte_accumulator) {
            Ok(s) => {
                emitted_piece.push_str(s);
                byte_accumulator.clear();
            }
            Err(e) => {
                let valid = e.valid_up_to();
                if valid > 0 {
                    let s = unsafe { std::str::from_utf8_unchecked(&byte_accumulator[..valid]) };
                    emitted_piece.push_str(s);
                    byte_accumulator.drain(..valid);
                }
            }
        }

        // Stop sequence matching
        if !emitted_piece.is_empty() {
            generated_text.push_str(&emitted_piece);
            let mut stopped = false;
            for stop_seq in &req.params.stop {
                if !stop_seq.is_empty() && generated_text.contains(stop_seq) {
                    if let Some(idx) = generated_text.find(stop_seq) {
                        let excess = generated_text.len() - idx;
                        if emitted_piece.len() >= excess {
                            emitted_piece.truncate(emitted_piece.len() - excess);
                        } else {
                            emitted_piece.clear();
                        }
                    }
                    stopped = true;
                    finish_reason = FinishReason::Stop;
                    break;
                }
            }

            if !emitted_piece.is_empty() {
                batch_text.push_str(&emitted_piece);
                batch_tokens += 1;
            }

            if stopped {
                break;
            }
        }

        // Batch coalescing: engines MUST coalesce to >=16ms or >=8 tokens per call
        if (batch_tokens >= 8 || last_emit.elapsed() >= Duration::from_millis(16))
            && !batch_text.is_empty()
        {
            let delta = GenerateDelta {
                text: std::mem::take(&mut batch_text),
                reasoning: String::new(),
                tool_calls: Vec::new(),
            };
            if tx.blocking_send(DecodeMessage::Delta(delta)).is_err() {
                // Receiver was dropped (e.g. caller cancelled / connection closed)
                return;
            }
            batch_tokens = 0;
            last_emit = Instant::now();
        }

        // Prepare batch for next step
        batch.clear();
        if let Err(e) = batch.add(token, current_pos, &[0], true) {
            let _ = tx.blocking_send(DecodeMessage::Error(PocketError::EngineFailed {
                engine: EngineKind::LlamaCpp,
                message: format!("failed adding token to batch: {e}"),
            }));
            return;
        }
        current_pos += 1;

        if let Err(e) = context.decode(&mut batch) {
            let _ = tx.blocking_send(DecodeMessage::Error(PocketError::EngineFailed {
                engine: EngineKind::LlamaCpp,
                message: format!("failed decoding token: {e}"),
            }));
            return;
        }
    }

    // Check if generation reached token or context limit
    if (completion_tokens >= max_tokens || (current_pos as u32) >= n_ctx)
        && finish_reason != FinishReason::Cancelled
    {
        finish_reason = FinishReason::Length;
    }

    // Flush any remaining buffered delta if not cancelled
    if !batch_text.is_empty() && finish_reason != FinishReason::Cancelled {
        let delta = GenerateDelta {
            text: std::mem::take(&mut batch_text),
            reasoning: String::new(),
            tool_calls: Vec::new(),
        };
        let _ = tx.blocking_send(DecodeMessage::Delta(delta));
    }

    let elapsed_after_first = first_token_time
        .map(|t| t.elapsed().as_secs_f32())
        .unwrap_or(0.0);
    let decode_tps = if elapsed_after_first > 0.0 {
        completion_tokens as f32 / elapsed_after_first
    } else {
        0.0
    };

    let stats = GenerateStats {
        prompt_tokens: total_prompt as u32,
        completion_tokens,
        ttft_ms,
        decode_tps,
        finish: finish_reason,
    };
    let _ = tx.blocking_send(DecodeMessage::Done(stats));
}
