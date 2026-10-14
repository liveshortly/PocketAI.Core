//! ENG-07's orchestrator: when a round finishes with `FinishReason::ToolCalls`,
//! parse the (Hermes-style, for now - see D43) tool calls out of that
//! round's text, execute them, append the exchange as plain chat turns
//! (`role: "assistant"` holds the raw tool-call text, `role: "tool"` holds
//! each result - exactly how these text-based tool-calling chat templates
//! round-trip, no new `ChatMessage` fields needed), and generate again.
//! Stops after 5 rounds, or immediately if the same tool call repeats
//! twice in a row (runaway guard).
use std::sync::{Arc, Mutex};

use pocket_types::{
    ChatMessage, ContentPart, FinishReason, GenerateDelta, GenerateRequest, GenerateStats,
    PocketError, ReasoningMode, SamplingParams, ToolSpec,
};

use crate::tools::builtin::ToolRegistry;
use crate::tools::parse::parse_tool_calls_hermes;
use crate::traits::{Engine, TokenSink};

const MAX_ROUNDS: u32 = 5;

struct TextCollectingSink {
    text: Mutex<String>,
}
impl TokenSink for TextCollectingSink {
    fn on_delta(&self, delta: GenerateDelta) {
        self.text.lock().unwrap().push_str(&delta.text);
    }
    fn on_done(&self, _stats: GenerateStats) {}
    fn on_error(&self, _err: PocketError) {}
}

pub async fn run_tool_loop(
    engine: Arc<dyn Engine>,
    handle: String,
    mut messages: Vec<ChatMessage>,
    params: SamplingParams,
    tool_specs: Vec<ToolSpec>,
    tools: &ToolRegistry,
    sink: Arc<dyn TokenSink>,
) -> Result<(Vec<ChatMessage>, GenerateStats), PocketError> {
    let mut last_signature: Option<String> = None;

    for round in 0..MAX_ROUNDS {
        let req = GenerateRequest {
            request_id: format!("toolloop-{round}"),
            messages: messages.clone(),
            tools: tool_specs.clone(),
            params: params.clone(),
            reasoning: ReasoningMode::Off,
        };
        let collector = Arc::new(TextCollectingSink {
            text: Mutex::new(String::new()),
        });
        let stats = engine
            .generate(handle.clone(), req, collector.clone())
            .await?;
        let text = collector.text.lock().unwrap().clone();

        if stats.finish != FinishReason::ToolCalls {
            sink.on_delta(GenerateDelta {
                text: text.clone(),
                reasoning: String::new(),
                tool_calls: vec![],
            });
            sink.on_done(stats.clone());
            messages.push(ChatMessage {
                role: "assistant".into(),
                parts: vec![ContentPart::Text { text }],
            });
            return Ok((messages, stats));
        }

        let calls = parse_tool_calls_hermes(&text);
        if calls.is_empty() {
            // The engine claimed tool_calls but nothing parsed out of the
            // text - stop rather than loop forever re-asking the same thing.
            sink.on_done(stats.clone());
            messages.push(ChatMessage {
                role: "assistant".into(),
                parts: vec![ContentPart::Text { text }],
            });
            return Ok((messages, stats));
        }

        let signature = calls
            .iter()
            .map(|c| format!("{}:{}", c.name, c.arguments_json))
            .collect::<Vec<_>>()
            .join("|");
        messages.push(ChatMessage {
            role: "assistant".into(),
            parts: vec![ContentPart::Text { text }],
        });

        if last_signature.as_deref() == Some(signature.as_str()) {
            let notice = "Repeated identical tool call detected; stopping.";
            messages.push(ChatMessage {
                role: "tool".into(),
                parts: vec![ContentPart::Text {
                    text: notice.into(),
                }],
            });
            let stopped = GenerateStats {
                finish: FinishReason::Stop,
                ..stats
            };
            sink.on_done(stopped.clone());
            return Ok((messages, stopped));
        }
        last_signature = Some(signature);

        for call in calls {
            let result = match tools.call(&call.name, &call.arguments_json) {
                Ok(s) => s,
                Err(e) => format!("Error: {e}"),
            };
            messages.push(ChatMessage {
                role: "tool".into(),
                parts: vec![ContentPart::Text { text: result }],
            });
        }
    }

    let stats = GenerateStats {
        prompt_tokens: 0,
        completion_tokens: 0,
        ttft_ms: 0,
        decode_tps: 0.0,
        finish: FinishReason::Length,
    };
    sink.on_done(stats.clone());
    Ok((messages, stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex as StdMutex;

    use pocket_types::{LoadOptions, LoadedModel, ModelManifest};

    /// Returns a scripted sequence of (text, finish) responses, one per
    /// call to `generate`, ignoring the actual request content - exactly
    /// what the card's "Done when: tests with scripted MockEngine outputs"
    /// asks for.
    struct ScriptedEngine {
        script: StdMutex<Vec<(String, FinishReason)>>,
        calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl Engine for ScriptedEngine {
        fn kind(&self) -> pocket_types::EngineKind {
            pocket_types::EngineKind::Mlx
        }
        fn supports(&self, _model: &ModelManifest) -> bool {
            true
        }
        async fn load(
            &self,
            _model: ModelManifest,
            _opts: LoadOptions,
        ) -> Result<LoadedModel, PocketError> {
            Ok(LoadedModel { handle: "h".into() })
        }
        async fn unload(&self, _handle: String) -> Result<(), PocketError> {
            Ok(())
        }
        async fn generate(
            &self,
            _handle: String,
            _req: GenerateRequest,
            sink: Arc<dyn TokenSink>,
        ) -> Result<GenerateStats, PocketError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut script = self.script.lock().unwrap();
            let (text, finish) = if script.is_empty() {
                (String::new(), FinishReason::Stop)
            } else {
                script.remove(0)
            };
            sink.on_delta(GenerateDelta {
                text,
                reasoning: String::new(),
                tool_calls: vec![],
            });
            let stats = GenerateStats {
                prompt_tokens: 1,
                completion_tokens: 1,
                ttft_ms: 1,
                decode_tps: 1.0,
                finish,
            };
            sink.on_done(stats.clone());
            Ok(stats)
        }
        async fn embed(
            &self,
            _handle: String,
            _inputs: Vec<String>,
        ) -> Result<Vec<Vec<f32>>, PocketError> {
            Ok(vec![])
        }
        fn cancel(&self, _request_id: String) {}
        fn resident_bytes(&self, _handle: String) -> u64 {
            0
        }
    }

    struct NullSink;
    impl TokenSink for NullSink {
        fn on_delta(&self, _delta: GenerateDelta) {}
        fn on_done(&self, _stats: GenerateStats) {}
        fn on_error(&self, _err: PocketError) {}
    }

    fn tool_call_text(name: &str, args: &str) -> String {
        format!(r#"<tool_call>{{"name": "{name}", "arguments": {args}}}</tool_call>"#)
    }

    #[tokio::test]
    async fn executes_a_tool_call_then_returns_the_final_answer() {
        let engine = Arc::new(ScriptedEngine {
            script: StdMutex::new(vec![
                (
                    tool_call_text("calculator", r#"{"expression": "2+2"}"#),
                    FinishReason::ToolCalls,
                ),
                ("The answer is 4.".into(), FinishReason::Stop),
            ]),
            calls: AtomicUsize::new(0),
        });
        let tools = ToolRegistry::with_builtins(|| 0);
        let messages = vec![ChatMessage {
            role: "user".into(),
            parts: vec![ContentPart::Text {
                text: "what is 2+2?".into(),
            }],
        }];

        let (messages, stats) = run_tool_loop(
            engine.clone(),
            "h".into(),
            messages,
            SamplingParams::default(),
            vec![],
            &tools,
            Arc::new(NullSink),
        )
        .await
        .unwrap();

        assert_eq!(stats.finish, FinishReason::Stop);
        assert_eq!(engine.calls.load(Ordering::SeqCst), 2);
        // user, assistant(tool call), tool(result "4"), assistant(final answer)
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[2].role, "tool");
        let ContentPart::Text { text } = &messages[2].parts[0] else {
            panic!()
        };
        assert_eq!(text, "4");
        assert_eq!(messages[3].role, "assistant");
    }

    #[tokio::test]
    async fn stops_without_executing_when_finish_is_not_tool_calls() {
        let engine = Arc::new(ScriptedEngine {
            script: StdMutex::new(vec![("just an answer".into(), FinishReason::Stop)]),
            calls: AtomicUsize::new(0),
        });
        let tools = ToolRegistry::new();
        let messages = vec![ChatMessage {
            role: "user".into(),
            parts: vec![ContentPart::Text { text: "hi".into() }],
        }];

        let (messages, stats) = run_tool_loop(
            engine.clone(),
            "h".into(),
            messages,
            SamplingParams::default(),
            vec![],
            &tools,
            Arc::new(NullSink),
        )
        .await
        .unwrap();

        assert_eq!(stats.finish, FinishReason::Stop);
        assert_eq!(engine.calls.load(Ordering::SeqCst), 1);
        assert_eq!(messages.len(), 2);
    }

    #[tokio::test]
    async fn runaway_guard_stops_after_the_same_call_repeats_twice() {
        let repeated = tool_call_text("calculator", r#"{"expression": "1+1"}"#);
        let engine = Arc::new(ScriptedEngine {
            script: StdMutex::new(vec![
                (repeated.clone(), FinishReason::ToolCalls),
                (repeated.clone(), FinishReason::ToolCalls),
                ("should never get here".into(), FinishReason::Stop),
            ]),
            calls: AtomicUsize::new(0),
        });
        let tools = ToolRegistry::with_builtins(|| 0);
        let messages = vec![ChatMessage {
            role: "user".into(),
            parts: vec![ContentPart::Text {
                text: "loop please".into(),
            }],
        }];

        let (messages, stats) = run_tool_loop(
            engine.clone(),
            "h".into(),
            messages,
            SamplingParams::default(),
            vec![],
            &tools,
            Arc::new(NullSink),
        )
        .await
        .unwrap();

        // Stopped on the 2nd identical call, never reaching the 3rd scripted response.
        assert_eq!(engine.calls.load(Ordering::SeqCst), 2);
        assert_eq!(stats.finish, FinishReason::Stop);
        assert_eq!(messages.last().unwrap().role, "tool");
    }

    #[tokio::test]
    async fn stops_after_max_rounds_of_tool_calls() {
        let call = tool_call_text("datetime", "{}");
        let mut script = Vec::new();
        for _ in 0..10 {
            script.push((call.clone(), FinishReason::ToolCalls));
        }
        let engine = Arc::new(ScriptedEngine {
            script: StdMutex::new(script),
            calls: AtomicUsize::new(0),
        });

        // Vary args each round in a fake way to dodge the runaway guard and
        // actually exercise the max-rounds limit.
        struct VaryingEngine(Arc<ScriptedEngine>, AtomicUsize);
        #[async_trait::async_trait]
        impl Engine for VaryingEngine {
            fn kind(&self) -> pocket_types::EngineKind {
                self.0.kind()
            }
            fn supports(&self, model: &ModelManifest) -> bool {
                self.0.supports(model)
            }
            async fn load(
                &self,
                model: ModelManifest,
                opts: LoadOptions,
            ) -> Result<LoadedModel, PocketError> {
                self.0.load(model, opts).await
            }
            async fn unload(&self, handle: String) -> Result<(), PocketError> {
                self.0.unload(handle).await
            }
            async fn generate(
                &self,
                handle: String,
                req: GenerateRequest,
                sink: Arc<dyn TokenSink>,
            ) -> Result<GenerateStats, PocketError> {
                let n = self.1.fetch_add(1, Ordering::SeqCst);
                let text = tool_call_text("datetime", &format!(r#"{{"round": {n}}}"#));
                sink.on_delta(GenerateDelta {
                    text,
                    reasoning: String::new(),
                    tool_calls: vec![],
                });
                let stats = GenerateStats {
                    prompt_tokens: 1,
                    completion_tokens: 1,
                    ttft_ms: 1,
                    decode_tps: 1.0,
                    finish: FinishReason::ToolCalls,
                };
                sink.on_done(stats.clone());
                let _ = req;
                let _ = handle;
                Ok(stats)
            }
            async fn embed(
                &self,
                handle: String,
                inputs: Vec<String>,
            ) -> Result<Vec<Vec<f32>>, PocketError> {
                self.0.embed(handle, inputs).await
            }
            fn cancel(&self, request_id: String) {
                self.0.cancel(request_id)
            }
            fn resident_bytes(&self, handle: String) -> u64 {
                self.0.resident_bytes(handle)
            }
        }

        let varying = Arc::new(VaryingEngine(engine, AtomicUsize::new(0)));
        let tools = ToolRegistry::with_builtins(|| 0);
        let messages = vec![ChatMessage {
            role: "user".into(),
            parts: vec![ContentPart::Text {
                text: "loop forever".into(),
            }],
        }];

        let (_messages, stats) = run_tool_loop(
            varying.clone(),
            "h".into(),
            messages,
            SamplingParams::default(),
            vec![],
            &tools,
            Arc::new(NullSink),
        )
        .await
        .unwrap();

        assert_eq!(stats.finish, FinishReason::Length);
        assert_eq!(varying.1.load(Ordering::SeqCst), MAX_ROUNDS as usize);
    }
}
