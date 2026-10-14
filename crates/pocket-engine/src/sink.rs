use std::sync::{Arc, Mutex};
use std::time::Duration;

use pocket_types::{GenerateDelta, GenerateStats, PocketError, ToolCallDelta};
use tokio::time::Instant;

use crate::traits::TokenSink;

const FLUSH_INTERVAL: Duration = Duration::from_millis(16);
const FLUSH_TOKEN_COUNT: usize = 8;

struct State {
    text: String,
    reasoning: String,
    tool_calls: Vec<ToolCallDelta>,
    token_count: usize,
    last_flush: Instant,
}

impl State {
    fn is_empty(&self) -> bool {
        self.text.is_empty() && self.reasoning.is_empty() && self.tool_calls.is_empty()
    }

    fn take_delta(&mut self) -> GenerateDelta {
        self.token_count = 0;
        self.last_flush = Instant::now();
        GenerateDelta {
            text: std::mem::take(&mut self.text),
            reasoning: std::mem::take(&mut self.reasoning),
            tool_calls: std::mem::take(&mut self.tool_calls),
        }
    }
}

/// Wraps any `TokenSink`, coalescing per-token deltas and flushing every 16ms or
/// 8 tokens, whichever is first. Always flushes on `on_done`/`on_error`.
pub struct BatchingSink {
    inner: Arc<dyn TokenSink>,
    state: Mutex<State>,
}

impl BatchingSink {
    pub fn new(inner: Arc<dyn TokenSink>) -> Self {
        Self {
            inner,
            state: Mutex::new(State {
                text: String::new(),
                reasoning: String::new(),
                tool_calls: Vec::new(),
                token_count: 0,
                last_flush: Instant::now(),
            }),
        }
    }

    /// Called once per generated token (or tool-call fragment).
    pub fn push(&self, delta: GenerateDelta) {
        let mut flushed = None;
        {
            let mut state = self.state.lock().unwrap();
            state.text.push_str(&delta.text);
            state.reasoning.push_str(&delta.reasoning);
            state.tool_calls.extend(delta.tool_calls);
            state.token_count += 1;

            let due = state.token_count >= FLUSH_TOKEN_COUNT
                || state.last_flush.elapsed() >= FLUSH_INTERVAL;
            if due && !state.is_empty() {
                flushed = Some(state.take_delta());
            }
        }
        if let Some(delta) = flushed {
            self.inner.on_delta(delta);
        }
    }

    fn flush_remaining(&self) {
        let flushed = {
            let mut state = self.state.lock().unwrap();
            if state.is_empty() {
                None
            } else {
                Some(state.take_delta())
            }
        };
        if let Some(delta) = flushed {
            self.inner.on_delta(delta);
        }
    }

    pub fn done(&self, stats: GenerateStats) {
        self.flush_remaining();
        self.inner.on_done(stats);
    }

    pub fn error(&self, err: PocketError) {
        self.flush_remaining();
        self.inner.on_error(err);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pocket_types::FinishReason;
    use std::sync::Mutex as StdMutex;

    struct RecordingSink {
        deltas: StdMutex<Vec<GenerateDelta>>,
        done: StdMutex<Option<GenerateStats>>,
    }

    impl RecordingSink {
        fn new() -> Self {
            Self {
                deltas: StdMutex::new(Vec::new()),
                done: StdMutex::new(None),
            }
        }
    }

    impl TokenSink for RecordingSink {
        fn on_delta(&self, delta: GenerateDelta) {
            self.deltas.lock().unwrap().push(delta);
        }
        fn on_done(&self, stats: GenerateStats) {
            *self.done.lock().unwrap() = Some(stats);
        }
        fn on_error(&self, _err: PocketError) {}
    }

    fn stats() -> GenerateStats {
        GenerateStats {
            prompt_tokens: 0,
            completion_tokens: 0,
            ttft_ms: 0,
            decode_tps: 0.0,
            finish: FinishReason::Stop,
        }
    }

    fn token(s: &str) -> GenerateDelta {
        GenerateDelta {
            text: s.to_string(),
            reasoning: String::new(),
            tool_calls: Vec::new(),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn flushes_at_eight_tokens() {
        let recording = Arc::new(RecordingSink::new());
        let batching = BatchingSink::new(recording.clone());
        for i in 0..8 {
            batching.push(token(&i.to_string()));
        }
        let deltas = recording.deltas.lock().unwrap();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].text, "01234567");
    }

    #[tokio::test(start_paused = true)]
    async fn flushes_after_16ms_even_with_few_tokens() {
        let recording = Arc::new(RecordingSink::new());
        let batching = BatchingSink::new(recording.clone());
        batching.push(token("a"));
        assert_eq!(recording.deltas.lock().unwrap().len(), 0);
        tokio::time::advance(Duration::from_millis(17)).await;
        batching.push(token("b"));
        let deltas = recording.deltas.lock().unwrap();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].text, "ab");
    }

    #[tokio::test(start_paused = true)]
    async fn flushes_remaining_on_done_without_losing_tokens() {
        let recording = Arc::new(RecordingSink::new());
        let batching = BatchingSink::new(recording.clone());
        batching.push(token("x"));
        batching.push(token("y"));
        batching.done(stats());

        let deltas = recording.deltas.lock().unwrap();
        let joined: String = deltas.iter().map(|d| d.text.clone()).collect();
        assert_eq!(joined, "xy");
        assert!(recording.done.lock().unwrap().is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn preserves_token_order_across_flushes() {
        let recording = Arc::new(RecordingSink::new());
        let batching = BatchingSink::new(recording.clone());
        for i in 0..20 {
            batching.push(token(&i.to_string()));
        }
        batching.done(stats());
        let deltas = recording.deltas.lock().unwrap();
        let joined: String = deltas.iter().map(|d| d.text.clone()).collect();
        let expected: String = (0..20).map(|i| i.to_string()).collect();
        assert_eq!(joined, expected);
    }
}
