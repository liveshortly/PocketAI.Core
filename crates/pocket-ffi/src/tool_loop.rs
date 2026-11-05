//! AGT-12 v1 step 2: the sync tool loop chat runs while a conversation's
//! Tools switch is on. Chat goes through the sync FFI `Engine` (the Swift MLX
//! engine registers as one), so `pocket_engine::run_tool_loop` (async engine
//! trait) can't be reused; this is its small sync twin.
//!
//! Each round: generate, split off reasoning, parse tool calls (Hermes, then
//! Llama 3 JSON). No calls -> that round is the answer. Otherwise run them,
//! append the assistant turn + one `tool` turn per result, and go again, up
//! to `MAX_ROUNDS`.
//!
//! Streaming: reasoning streams live every round. Visible text streams live
//! once a round's text clearly isn't a tool call (it doesn't open with
//! `<tool_call>` or `{`); a round that opens like a call is held, and if it
//! turns out to be the answer after all, it's replayed to the sink in one
//! delta at the end. `on_done` fires once, after the final round.
use std::sync::{Arc, Mutex};

use pocket_engine::{
    parse_tool_calls_hermes, parse_tool_calls_llama3, ParsedToolCall, ReasoningSplitter,
    ToolRegistry,
};

use crate::error::PocketError;
use crate::traits::{ChatSink, Engine, TokenSink, ToolExecutor};
use crate::types::*;

pub(crate) const MAX_ROUNDS: usize = 15;
/// AGT-13: a "Run this?" card nobody answers is a no after this long.
pub(crate) const APPROVAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);
/// AGT-13: the sandbox's hard run limit (Swift kills the script).
const RUN_TIMEOUT_MS: u32 = 10_000;
/// What the model reads when the user says no (or never answers).
pub(crate) const DENIED_RESULT: &str = "The user chose not to run this.";

const CALL_OPEN: &str = "<tool_call>";

/// Told to the model (system prompt) when Tools is on, so it stops claiming
/// it can't look things up.
pub(crate) const TOOLS_PROMPT: &str = "You can search the web with the web_search tool and read a page with fetch_url. Use them for current events, recent releases, prices or facts you are unsure of, then answer from what you read. Use calculator for arithmetic and datetime for the current date and time.";
/// AGT-13: added to the prompt when `run_js` and/or `run_python` is offered.
pub(crate) fn code_prompt(js: bool, py: bool, mac: bool) -> String {
    let mut text = local_code_prompt(js, py);
    if mac {
        let own = if text.is_empty() { "" } else { " " };
        text = format!("{text}{own}You can also run python, javascript or shell code on the user's Mac with the run_on_mac tool (sandboxed: no network, no access to their files, 10 second limit). Use it for work that needs a real machine; the user approves each run and may decline.");
    }
    text
}

/// UI-145: small models write a script and then invent its output. Say plainly
/// what to do instead, and what the sandbox cannot do.
pub(crate) const NEVER_FAKE_OUTPUT: &str = "When the user asks you to run code, CALL the tool; do not just show the code and tell them to run it themselves. Never write the output of code yourself: call the tool and report what it returned. The sandbox has no subprocess, no network and no files, so do not write code that needs them (no subprocess, ping, requests or open()).";

fn local_code_prompt(js: bool, py: bool) -> String {
    if !js && !py {
        return String::new();
    }
    let what = match (js, py) {
        (true, true) => "JavaScript with run_js or Python with run_python",
        (true, false) => "JavaScript with the run_js tool",
        _ => "Python with the run_python tool",
    };
    let output = match (js, py) {
        (true, false) => "use console.log or print for output, the last expression is returned too",
        (false, true) => "use print for output, the last expression is returned too, standard library only",
        _ => "print the results, the last expression is returned too; Python has the standard library only",
    };
    format!("You can run {what} on this device (no network, no files; {output}). {NEVER_FAKE_OUTPUT} Use it for exact arithmetic, parsing and data wrangling instead of working them out by hand. The user approves each run and may decline; if they do, answer without it.")
}
/// Told to the model when Tools is on but the device is offline.
pub(crate) const OFFLINE_PROMPT: &str = "Web search is unavailable right now because the device is offline. Answer from what you know and say that you couldn't search.";

fn spec(name: &str, description: &str, schema: serde_json::Value) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        description: description.into(),
        parameters_json_schema: schema.to_string(),
    }
}

fn string_arg(key: &str, description: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": { key: { "type": "string", "description": description } },
        "required": [key],
    })
}

/// `web`: web_search + fetch_url; `js`: run_js; `py`: run_python (AGT-13).
/// calculator and datetime are always on while Tools is.
pub(crate) fn tool_specs(web: bool, js: bool, py: bool, mac: bool) -> Vec<ToolSpec> {
    let mut specs = Vec::new();
    if web {
        specs.push(spec(
            "web_search",
            "Search the web. Returns the top results as title, url and snippet.",
            string_arg("query", "What to search for"),
        ));
        specs.push(spec(
            "fetch_url",
            "Read a web page and return its text.",
            string_arg("url", "The http(s) URL to read"),
        ));
    }
    if js {
        specs.push(spec(
            "run_js",
            "Run JavaScript on this device in a sandbox (no network, no files, 10 second limit). The user must approve each run. Print results with console.log.",
            string_arg("code", "The JavaScript to run"),
        ));
    }
    if py {
        specs.push(spec(
            "run_python",
            "Run Python on this device in a sandbox (standard library only, no network, no files, 10 second limit). The user must approve each run. Print results with print().",
            string_arg("code", "The Python to run"),
        ));
    }
    if mac {
        specs.push(spec(
            RUN_ON_MAC,
            "Run code on the user's Mac in a sandbox (no network, no access to their files, 10 second limit). The user must approve each run. Print results to stdout.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "language": { "type": "string", "enum": ["python", "javascript", "shell"], "description": "python, javascript or shell (zsh)" },
                    "code": { "type": "string", "description": "The code to run" },
                },
                "required": ["language", "code"],
            }),
        ));
    }
    specs.extend(base_specs());
    specs
}

fn base_specs() -> Vec<ToolSpec> {
    vec![
        spec(
            "calculator",
            "Evaluate an arithmetic expression.",
            string_arg("expression", "For example 2 * (3 + 4)"),
        ),
        spec(
            "datetime",
            "The current date and time (UTC, ISO 8601).",
            serde_json::json!({ "type": "object", "properties": {} }),
        ),
    ]
}

/// calculator, datetime, web_search, fetch_url over `fetch`.
pub(crate) fn registry(
    now_ms: impl Fn() -> i64 + Send + Sync + 'static,
    fetch: Box<dyn Fn(&str) -> Result<String, String> + Send + Sync>,
) -> ToolRegistry {
    let mut registry = ToolRegistry::with_builtins(now_ms);
    pocket_engine::register_web_tools(&mut registry, fetch);
    registry
}

/// The real network: `reqwest::blocking` on its own `std::thread`, joined,
/// so it never blocks inside (or nests) a tokio runtime. Each fetch is
/// recorded in `egress` for the privacy log.
pub(crate) fn http_fetcher(
    egress: Arc<Mutex<Vec<EgressEntry>>>,
    now_ms: impl Fn() -> i64 + Send + Sync + 'static,
) -> Box<dyn Fn(&str) -> Result<String, String> + Send + Sync> {
    Box::new(move |url: &str| {
        let owned = url.to_string();
        let body = std::thread::spawn(move || -> Result<String, String> {
            let client = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15")
                .build()
                .map_err(|e| e.to_string())?;
            let resp = client.get(&owned).send().map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("HTTP {}", resp.status()));
            }
            resp.text().map_err(|e| e.to_string())
        })
        .join()
        .map_err(|_| "the request failed".to_string())??;
        egress.lock().unwrap().push(EgressEntry {
            ts: now_ms(),
            host: host_of(url),
            purpose: "web search".into(),
            bytes_out: 0,
            bytes_in: body.len() as u64,
            initiated_by: "user".into(),
        });
        Ok(body)
    })
}

fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    host.trim_start_matches("www.").to_string()
}

pub(crate) struct LoopOutput {
    pub stats: GenerateStats,
    pub text: String,
    pub reasoning: String,
    pub steps: Vec<ToolStep>,
    pub sources: Vec<String>,
}

#[derive(PartialEq)]
enum Gate {
    Undecided,
    Live,
    Held,
}

/// One round's sink: splits reasoning, streams it live, gates visible text.
struct RoundSink {
    out: Arc<dyn ChatSink>,
    splitter: Mutex<ReasoningSplitter>,
    text: Mutex<String>,
    reasoning: Mutex<String>,
    gate: Mutex<Gate>,
}

impl RoundSink {
    fn new(out: Arc<dyn ChatSink>) -> Self {
        Self {
            out,
            splitter: Mutex::new(ReasoningSplitter::new()),
            text: Mutex::new(String::new()),
            reasoning: Mutex::new(String::new()),
            gate: Mutex::new(Gate::Undecided),
        }
    }

    fn emit(&self, text: String, reasoning: String) {
        if text.is_empty() && reasoning.is_empty() {
            return;
        }
        self.out.on_delta(GenerateDelta {
            text,
            reasoning,
            tool_calls: vec![],
        });
    }

    fn take(&self, text: &str, reasoning: &str) {
        self.reasoning.lock().unwrap().push_str(reasoning);
        let mut all = self.text.lock().unwrap();
        all.push_str(text);
        let mut gate = self.gate.lock().unwrap();
        let shown = match *gate {
            Gate::Live => text.to_string(),
            Gate::Held => String::new(),
            Gate::Undecided => {
                let t = all.trim_start();
                if t.is_empty() || CALL_OPEN.starts_with(t) {
                    String::new()
                } else if t.starts_with(CALL_OPEN) || t.starts_with('{') {
                    *gate = Gate::Held;
                    String::new()
                } else {
                    *gate = Gate::Live;
                    all.clone()
                }
            }
        };
        drop((gate, all));
        self.emit(shown, reasoning.to_string());
    }
}

impl TokenSink for RoundSink {
    fn on_delta(&self, delta: GenerateDelta) {
        if !delta.reasoning.is_empty() {
            self.take(&delta.text, &delta.reasoning);
            return;
        }
        let split = self.splitter.lock().unwrap().push(&delta.text);
        self.take(&split.text, &split.reasoning);
    }
    fn on_done(&self, _stats: GenerateStats) {
        let split = self.splitter.lock().unwrap().flush();
        self.take(&split.text, &split.reasoning);
    }
    fn on_error(&self, err: PocketError) {
        self.out.on_error(err);
    }
}

fn parse_calls(text: &str, registry: &ToolRegistry, code: Option<&CodeTools>) -> Vec<ParsedToolCall> {
    let calls = parse_tool_calls_hermes(text);
    if !calls.is_empty() {
        return calls;
    }
    // A bare JSON answer isn't a call unless it names one of our tools.
    parse_tool_calls_llama3(text)
        .into_iter()
        .filter(|c| registry.has(&c.name) || code.is_some_and(|t| t.offers(&c.name)))
        .collect()
}

const RUN_JS: &str = "run_js";
const RUN_PYTHON: &str = "run_python";
const RUN_ON_MAC: &str = "run_on_mac";

/// AGT-13: what `run_js` needs beyond the registry. `approve` blocks until the
/// user answers (or the wait limit / Stop says no); `preapproved` is the
/// per-chat "Always allow JavaScript" memory.
pub(crate) struct CodeTools<'a> {
    /// Which of `run_js` / `run_python` this reply offers.
    pub javascript: bool,
    pub python: bool,
    /// `run_on_mac`: a Mac is paired and connected (AGT-13 stage 2).
    pub mac: bool,
    pub executor: &'a dyn ToolExecutor,
    pub conversation_id: &'a str,
    pub approve: &'a dyn Fn(ApprovalRequest) -> ApprovalDecision,
    pub preapproved: &'a dyn Fn() -> bool,
}

impl CodeTools<'_> {
    /// Whether the model may call `name` this turn.
    pub(crate) fn offers(&self, name: &str) -> bool {
        (name == RUN_JS && self.javascript)
            || (name == RUN_PYTHON && self.python)
            || (name == RUN_ON_MAC && self.mac)
    }
}

/// Waits for the user's answer on `rx`. Anything but an explicit yes is a no:
/// the wait limit, Stop (`stopped`, polled), or a dropped channel.
pub(crate) fn await_decision(
    rx: &std::sync::mpsc::Receiver<ApprovalDecision>,
    limit: std::time::Duration,
    stopped: &dyn Fn() -> bool,
) -> ApprovalDecision {
    let start = std::time::Instant::now();
    let tick = std::time::Duration::from_millis(100).min(limit);
    while start.elapsed() < limit && !stopped() {
        match rx.recv_timeout(tick) {
            Ok(decision) => return decision,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    ApprovalDecision::Deny
}

/// Approval, then the run. `code` only reaches the executor from the `Run`
/// and `RunEdited` arms (or, JavaScript only, after the user's own "always
/// allow"; Python always asks).
fn run_code_tool(call: &ParsedToolCall, tools: &CodeTools) -> (String, ToolStep) {
    let mac = call.name == RUN_ON_MAC;
    let (language, place) = if mac {
        let lang = arg(call, "language").to_lowercase();
        match lang.as_str() {
            "python" => ("mac:python", "Run Python on your Mac"),
            "javascript" => ("mac:javascript", "Run JavaScript on your Mac"),
            "shell" => ("mac:shell", "Run Shell on your Mac"),
            _ => {
                return (
                    "Error: run_on_mac needs language to be python, javascript or shell.".into(),
                    ToolStep {
                        kind: "ran".into(),
                        label: "mac".into(),
                        detail: serde_json::json!({ "code": arg(call, "code"), "error": "Unknown language.", "target": "mac" }).to_string(),
                    },
                )
            }
        }
    } else if call.name == RUN_PYTHON {
        ("python", "Run Python on this iPhone")
    } else {
        ("javascript", "Run JavaScript on this iPhone")
    };
    let code = arg(call, "code");
    let step = |mut detail: serde_json::Value| {
        if mac {
            detail["target"] = "mac".into();
        }
        ToolStep {
            kind: "ran".into(),
            label: language.into(),
            detail: detail.to_string(),
        }
    };
    if code.trim().is_empty() {
        return (
            format!("Error: {} needs a non-empty code argument.", call.name),
            step(serde_json::json!({ "code": code, "error": "No code was given." })),
        );
    }
    // Only plain on-device JavaScript can be "always allowed"; Python and
    // everything on the Mac asks every single time.
    let decision = if language == "javascript" && (tools.preapproved)() {
        ApprovalDecision::Run
    } else {
        (tools.approve)(ApprovalRequest {
            id: String::new(), // minted by the approver
            conversation_id: tools.conversation_id.into(),
            tool: call.name.clone(),
            summary: place.into(),
            code: code.clone(),
        })
    };
    let code = match decision {
        ApprovalDecision::Run => code,
        ApprovalDecision::RunEdited { code } => code,
        ApprovalDecision::Deny => {
            return (
                DENIED_RESULT.into(),
                step(serde_json::json!({ "code": code, "denied": true })),
            )
        }
    };
    let res = tools.executor.run_code(language.into(), code.clone(), RUN_TIMEOUT_MS);
    let mut text = res.output.clone();
    if res.truncated {
        text.push_str("\n[output truncated]");
    }
    if !res.error.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&format!("Error: {}", res.error));
    }
    if text.is_empty() {
        text = "(no output)".into();
    }
    let detail = serde_json::json!({
        "code": code,
        "output": res.output,
        "error": res.error,
        "duration_ms": res.duration_ms,
        "truncated": res.truncated,
    });
    (text, step(detail))
}

fn arg(call: &ParsedToolCall, key: &str) -> String {
    serde_json::from_str::<serde_json::Value>(&call.arguments_json)
        .ok()
        .and_then(|v| v.get(key).and_then(|s| s.as_str()).map(str::to_string))
        .unwrap_or_default()
}

/// The visible row for a web tool run (calculator/datetime show none).
fn step_for(call: &ParsedToolCall, result: &Result<String, String>) -> Option<ToolStep> {
    match call.name.as_str() {
        "web_search" => Some(ToolStep {
            kind: "searched".into(),
            label: arg(call, "query"),
            detail: match result {
                Ok(json) => serde_json::from_str::<Vec<serde_json::Value>>(json)
                    .map(|rs| {
                        rs.iter()
                            .map(|r| {
                                format!(
                                    "{}\n{}",
                                    r["title"].as_str().unwrap_or(""),
                                    r["url"].as_str().unwrap_or("")
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n\n")
                    })
                    .unwrap_or_default(),
                Err(e) => format!("Couldn't search: {e}"),
            },
        }),
        "fetch_url" => {
            let url = arg(call, "url");
            Some(ToolStep {
                kind: "read".into(),
                label: host_of(&url),
                detail: match result {
                    Ok(_) => url,
                    Err(e) => format!("{url}\nCouldn't read: {e}"),
                },
            })
        }
        _ => None,
    }
}

/// Runs the loop. `req` carries the history, params and tool specs; the
/// same `request_id` is used every round so Stop's `engine.cancel` reaches
/// whichever round is running, and `cancelled` covers the gaps between
/// rounds (tool runs).
pub(crate) fn run(
    engine: &Arc<dyn Engine>,
    handle: &str,
    mut req: GenerateRequest,
    registry: &ToolRegistry,
    sink: &Arc<dyn ChatSink>,
    cancelled: &dyn Fn() -> bool,
    code: Option<&CodeTools>,
) -> Result<LoopOutput, PocketError> {
    let mut steps = Vec::new();
    let mut sources: Vec<String> = Vec::new();
    let mut reasoning = String::new();
    for round in 0..MAX_ROUNDS {
        let round_sink = Arc::new(RoundSink::new(Arc::clone(sink)));
        let stats = engine.generate(
            handle.to_string(),
            req.clone(),
            round_sink.clone() as Arc<dyn TokenSink>,
        )?;
        let text = round_sink.text.lock().unwrap().clone();
        reasoning.push_str(&round_sink.reasoning.lock().unwrap());
        let stopped = matches!(stats.finish, FinishReason::Cancelled) || cancelled();
        let calls = if stopped {
            vec![]
        } else {
            parse_calls(&text, registry, code)
        };
        let last = round + 1 == MAX_ROUNDS;
        if calls.is_empty() || stopped || last {
            let gate_live = *round_sink.gate.lock().unwrap() == Gate::Live;
            let text = if !calls.is_empty() {
                // ponytail: the cap ends the turn with a note instead of one
                // more tools-off round; raise MAX_ROUNDS if real chats hit it.
                format!("I stopped after {MAX_ROUNDS} tool steps without reaching an answer.")
            } else if stopped && !gate_live {
                String::new()
            } else {
                text
            };
            if !gate_live && !text.is_empty() {
                round_sink.emit(text.clone(), String::new());
            }
            let stats = GenerateStats {
                finish: if stopped {
                    FinishReason::Cancelled
                } else {
                    stats.finish
                },
                ..stats
            };
            sink.on_done(stats.clone());
            return Ok(LoopOutput {
                stats,
                text,
                reasoning,
                steps,
                sources,
            });
        }

        req.messages.push(ChatMessage {
            role: "assistant".into(),
            parts: vec![ContentPart::Text { text }],
        });
        for call in calls {
            if let Some(tools) = code.filter(|t| t.offers(&call.name)) {
                let (text, step) = if cancelled() {
                    (DENIED_RESULT.to_string(), None)
                } else {
                    let (t, s) = run_code_tool(&call, tools);
                    (t, Some(s))
                };
                if let Some(step) = step {
                    sink.on_tool_step(step.clone());
                    steps.push(step);
                }
                req.messages.push(ChatMessage {
                    role: "tool".into(),
                    parts: vec![ContentPart::Text { text }],
                });
                if cancelled() {
                    break;
                }
                continue;
            }
            let result = registry.call(&call.name, &call.arguments_json);
            if let Some(step) = step_for(&call, &result) {
                if step.kind == "read" && result.is_ok() {
                    let url = arg(&call, "url");
                    if !sources.contains(&url) {
                        sources.push(url);
                    }
                }
                sink.on_tool_step(step.clone());
                steps.push(step);
            }
            req.messages.push(ChatMessage {
                role: "tool".into(),
                parts: vec![ContentPart::Text {
                    text: result.unwrap_or_else(|e| format!("Error: {e}")),
                }],
            });
            if cancelled() {
                break;
            }
        }
    }
    unreachable!("the last round always returns")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Replies with `script[i]` on call i (the last entry repeats).
    struct ScriptEngine {
        script: Vec<&'static str>,
        calls: AtomicUsize,
        seen: Mutex<Vec<GenerateRequest>>,
    }

    impl Engine for ScriptEngine {
        fn kind(&self) -> EngineKind {
            EngineKind::Mlx
        }
        fn supports(&self, _model: ModelManifest) -> bool {
            true
        }
        fn load(&self, _m: ModelManifest, _o: LoadOptions) -> Result<LoadedModel, PocketError> {
            unimplemented!()
        }
        fn unload(&self, _handle: String) -> Result<(), PocketError> {
            Ok(())
        }
        fn generate(
            &self,
            _handle: String,
            req: GenerateRequest,
            sink: Arc<dyn TokenSink>,
        ) -> Result<GenerateStats, PocketError> {
            let i = self.calls.fetch_add(1, Ordering::SeqCst);
            self.seen.lock().unwrap().push(req);
            let text = self.script[i.min(self.script.len() - 1)];
            // Two deltas, so the gate sees a split stream.
            let (a, b) = text.split_at(text.len() / 2);
            for t in [a, b] {
                sink.on_delta(GenerateDelta {
                    text: t.into(),
                    reasoning: String::new(),
                    tool_calls: vec![],
                });
            }
            let stats = GenerateStats {
                prompt_tokens: 1,
                completion_tokens: 2,
                ttft_ms: 0,
                decode_tps: 0.0,
                finish: FinishReason::Stop,
            };
            sink.on_done(stats.clone());
            Ok(stats)
        }
        fn embed(&self, _h: String, _i: Vec<String>) -> Result<Vec<Vec<f32>>, PocketError> {
            Ok(vec![])
        }
        fn cancel(&self, _request_id: String) {}
        fn resident_bytes(&self, _handle: String) -> u64 {
            0
        }
    }

    #[derive(Default)]
    struct Recorder {
        text: Mutex<String>,
        steps: Mutex<Vec<ToolStep>>,
        done: AtomicUsize,
    }

    impl ChatSink for Recorder {
        fn on_delta(&self, delta: GenerateDelta) {
            self.text.lock().unwrap().push_str(&delta.text);
        }
        fn on_done(&self, _stats: GenerateStats) {
            self.done.fetch_add(1, Ordering::SeqCst);
        }
        fn on_error(&self, _err: PocketError) {}
        fn on_message_saved(&self, _message: Message) {}
        fn on_tool_step(&self, step: ToolStep) {
            self.steps.lock().unwrap().push(step);
        }
    }

    const DDG: &str = r#"<div class="result results_links web-result"><a rel="nofollow" class="result__a" href="https://www.apple.com/macos/">macOS Tahoe - Apple</a><a class="result__snippet" href="https://www.apple.com/macos/">The latest macOS.</a></div>"#;

    fn fake_registry() -> ToolRegistry {
        registry(
            || 0,
            Box::new(|url: &str| {
                if url.contains("duckduckgo") {
                    Ok(DDG.to_string())
                } else {
                    Ok("<html><body><p>macOS 26 Tahoe is the latest.</p></body></html>".into())
                }
            }),
        )
    }

    fn request() -> GenerateRequest {
        GenerateRequest {
            request_id: "r1".into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                parts: vec![ContentPart::Text {
                    text: "latest macOS?".into(),
                }],
            }],
            tools: tool_specs(true, false, false, false),
            params: SamplingParams {
                temperature: 0.0,
                top_p: 1.0,
                top_k: 0,
                min_p: 0.0,
                max_tokens: 64,
                stop: vec![],
                seed: None,
                repetition_penalty: 0.0,
            },
            reasoning: ReasoningMode::Off,
        }
    }

    fn engine(script: Vec<&'static str>) -> Arc<ScriptEngine> {
        Arc::new(ScriptEngine {
            script,
            calls: AtomicUsize::new(0),
            seen: Mutex::new(vec![]),
        })
    }

    #[test]
    fn a_hermes_call_runs_the_tool_then_streams_only_the_answer() {
        let eng = engine(vec![
            "<think>need to look</think><tool_call>{\"name\": \"web_search\", \"arguments\": {\"query\": \"latest macOS\"}}</tool_call>",
            "<tool_call>{\"name\": \"fetch_url\", \"arguments\": {\"url\": \"https://www.apple.com/macos/\"}}</tool_call>",
            "The latest is macOS 26 Tahoe.",
        ]);
        let rec = Arc::new(Recorder::default());
        let sink: Arc<dyn ChatSink> = rec.clone();
        let dyn_eng: Arc<dyn Engine> = eng.clone();
        let out = run(&dyn_eng, "h", request(), &fake_registry(), &sink, &|| false, None).unwrap();

        assert_eq!(out.text, "The latest is macOS 26 Tahoe.");
        assert_eq!(*rec.text.lock().unwrap(), "The latest is macOS 26 Tahoe.");
        assert_eq!(rec.done.load(Ordering::SeqCst), 1);
        assert_eq!(out.reasoning, "need to look");
        let steps = rec.steps.lock().unwrap().clone();
        assert_eq!(steps, out.steps);
        assert_eq!(steps[0].kind, "searched");
        assert_eq!(steps[0].label, "latest macOS");
        assert!(steps[0].detail.contains("https://www.apple.com/macos/"));
        assert_eq!(steps[1].kind, "read");
        assert_eq!(steps[1].label, "apple.com");
        assert_eq!(
            out.sources,
            vec!["https://www.apple.com/macos/".to_string()]
        );

        // Round 3 saw both exchanges as assistant + tool turns.
        let seen = eng.seen.lock().unwrap();
        let roles: Vec<_> = seen[2].messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, ["user", "assistant", "tool", "assistant", "tool"]);
        assert_eq!(seen[0].tools.len(), 4);
        assert!(seen[0].tools.iter().all(|t| t.name != "run_js"));
    }

    #[test]
    fn a_failing_tool_returns_its_error_to_the_model() {
        let eng = engine(vec![
            "<tool_call>{\"name\": \"web_search\", \"arguments\": {\"query\": \"x\"}}</tool_call>",
            "I couldn't search.",
        ]);
        let offline = registry(|| 0, Box::new(|_: &str| Err("network down".into())));
        let rec = Arc::new(Recorder::default());
        let sink: Arc<dyn ChatSink> = rec.clone();
        let dyn_eng: Arc<dyn Engine> = eng.clone();
        let out = run(&dyn_eng, "h", request(), &offline, &sink, &|| false, None).unwrap();
        assert_eq!(out.text, "I couldn't search.");
        let seen = eng.seen.lock().unwrap();
        let tool_turn = &seen[1].messages[2];
        assert!(
            matches!(&tool_turn.parts[0], ContentPart::Text { text } if text.contains("network down"))
        );
        assert!(out.steps[0].detail.contains("network down"));
    }

    #[test]
    fn the_round_cap_holds() {
        let eng = engine(vec![
            "<tool_call>{\"name\": \"datetime\", \"arguments\": {}}</tool_call>",
        ]);
        let rec = Arc::new(Recorder::default());
        let sink: Arc<dyn ChatSink> = rec.clone();
        let dyn_eng: Arc<dyn Engine> = eng.clone();
        let out = run(&dyn_eng, "h", request(), &fake_registry(), &sink, &|| false, None).unwrap();
        assert_eq!(eng.calls.load(Ordering::SeqCst), MAX_ROUNDS);
        assert!(out.text.contains("stopped"));
        assert_eq!(rec.done.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn stop_between_rounds_ends_the_turn() {
        let eng = engine(vec![
            "<tool_call>{\"name\": \"datetime\", \"arguments\": {}}</tool_call>",
        ]);
        let rec = Arc::new(Recorder::default());
        let sink: Arc<dyn ChatSink> = rec.clone();
        let dyn_eng: Arc<dyn Engine> = eng.clone();
        let out = run(
            &dyn_eng,
            "h",
            request(),
            &fake_registry(),
            &sink,
            &|| eng.calls.load(Ordering::SeqCst) >= 2,
            None,
        )
        .unwrap();
        assert_eq!(eng.calls.load(Ordering::SeqCst), 2);
        assert!(matches!(out.stats.finish, FinishReason::Cancelled));
        assert_eq!(out.text, "");
    }

    #[test]
    fn a_plain_answer_streams_live() {
        let eng = engine(vec!["Hello there, no tools needed."]);
        let rec = Arc::new(Recorder::default());
        let sink: Arc<dyn ChatSink> = rec.clone();
        let dyn_eng: Arc<dyn Engine> = eng.clone();
        let out = run(&dyn_eng, "h", request(), &fake_registry(), &sink, &|| false, None).unwrap();
        assert_eq!(out.text, "Hello there, no tools needed.");
        assert_eq!(*rec.text.lock().unwrap(), out.text);
        assert!(out.steps.is_empty());
    }

    // ---- AGT-13: run_js ----

    struct MockExecutor {
        runs: Mutex<Vec<(String, String, u32)>>,
        result: ToolRunResult,
    }

    impl MockExecutor {
        fn new() -> Self {
            Self {
                runs: Mutex::new(vec![]),
                result: ToolRunResult {
                    ok: true,
                    output: "42".into(),
                    error: String::new(),
                    duration_ms: 3,
                    truncated: false,
                },
            }
        }
    }

    impl ToolExecutor for MockExecutor {
        fn availability(&self) -> ToolAvailability {
            ToolAvailability { web: true, javascript: true, python: true, mac: false }
        }
        fn run_code(&self, language: String, code: String, timeout_ms: u32) -> ToolRunResult {
            self.runs.lock().unwrap().push((language, code, timeout_ms));
            self.result.clone()
        }
        fn request_approval(&self, _r: ApprovalRequest) {}
        fn withdraw_approval(&self, _id: String) {}
        fn cancel_run(&self) {}
    }

    const JS_CALL: &str =
        "<tool_call>{\"name\": \"run_js\", \"arguments\": {\"code\": \"6*7\"}}</tool_call>";

    /// Runs the loop with run_js on; `decide` answers every approval.
    fn run_with_js(
        script: Vec<&'static str>,
        exec: &MockExecutor,
        decide: &dyn Fn(ApprovalRequest) -> ApprovalDecision,
        preapproved: bool,
    ) -> (LoopOutput, Arc<ScriptEngine>, Arc<Recorder>) {
        run_with(script, exec, decide, preapproved, (true, false, false))
    }

    /// `(js, py, mac)`: which code tools the reply offers.
    fn run_with(
        script: Vec<&'static str>,
        exec: &MockExecutor,
        decide: &dyn Fn(ApprovalRequest) -> ApprovalDecision,
        preapproved: bool,
        (js, py, mac): (bool, bool, bool),
    ) -> (LoopOutput, Arc<ScriptEngine>, Arc<Recorder>) {
        let eng = engine(script);
        let rec = Arc::new(Recorder::default());
        let sink: Arc<dyn ChatSink> = rec.clone();
        let dyn_eng: Arc<dyn Engine> = eng.clone();
        let pre = move || preapproved;
        let tools = CodeTools {
            javascript: js,
            python: py,
            mac,
            executor: exec,
            conversation_id: "c1",
            approve: decide,
            preapproved: &pre,
        };
        let mut req = request();
        req.tools = tool_specs(false, js, py, false);
        let out = run(&dyn_eng, "h", req, &fake_registry(), &sink, &|| false, Some(&tools)).unwrap();
        (out, eng, rec)
    }

    fn tool_turn(eng: &ScriptEngine) -> String {
        let seen = eng.seen.lock().unwrap();
        match &seen[1].messages.last().unwrap().parts[0] {
            ContentPart::Text { text } => text.clone(),
            _ => panic!("not text"),
        }
    }

    #[test]
    fn approved_code_runs_and_the_model_reads_the_output() {
        let exec = MockExecutor::new();
        let asked = Mutex::new(vec![]);
        let (out, eng, rec) = run_with_js(
            vec![JS_CALL, "It is 42."],
            &exec,
            &|r| {
                asked.lock().unwrap().push(r);
                ApprovalDecision::Run
            },
            false,
        );
        assert_eq!(out.text, "It is 42.");
        assert_eq!(asked.lock().unwrap()[0].code, "6*7");
        assert_eq!(
            *exec.runs.lock().unwrap(),
            vec![("javascript".to_string(), "6*7".to_string(), 10_000)]
        );
        assert_eq!(tool_turn(&eng), "42");
        let step = &rec.steps.lock().unwrap()[0];
        assert_eq!(step.kind, "ran");
        assert_eq!(step.label, "javascript");
        let d: serde_json::Value = serde_json::from_str(&step.detail).unwrap();
        assert_eq!((d["code"].as_str(), d["output"].as_str(), d["duration_ms"].as_u64()), (Some("6*7"), Some("42"), Some(3)));
        assert_eq!(out.steps.len(), 1);
    }

    #[test]
    fn edited_code_is_what_runs() {
        let exec = MockExecutor::new();
        let (_, _, rec) = run_with_js(
            vec![JS_CALL, "ok"],
            &exec,
            &|_| ApprovalDecision::RunEdited { code: "1+1".into() },
            false,
        );
        assert_eq!(exec.runs.lock().unwrap()[0].1, "1+1");
        assert!(rec.steps.lock().unwrap()[0].detail.contains("1+1"));
    }

    #[test]
    fn denied_code_never_runs() {
        let exec = MockExecutor::new();
        let (out, eng, rec) =
            run_with_js(vec![JS_CALL, "Fine, by hand."], &exec, &|_| ApprovalDecision::Deny, false);
        assert!(exec.runs.lock().unwrap().is_empty());
        assert_eq!(tool_turn(&eng), "The user chose not to run this.");
        assert_eq!(out.text, "Fine, by hand.");
        assert!(rec.steps.lock().unwrap()[0].detail.contains("\"denied\":true"));
    }

    #[test]
    fn always_allow_skips_the_card_but_still_runs_in_the_sandbox() {
        let exec = MockExecutor::new();
        let (_, _, _) = run_with_js(
            vec![JS_CALL, "ok"],
            &exec,
            &|_| panic!("no card expected"),
            true,
        );
        assert_eq!(exec.runs.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_failed_run_reaches_the_model_as_an_error() {
        let mut exec = MockExecutor::new();
        exec.result = ToolRunResult {
            ok: false,
            output: String::new(),
            error: "ReferenceError: x is not defined".into(),
            duration_ms: 1,
            truncated: false,
        };
        let (_, eng, _) = run_with_js(vec![JS_CALL, "sorry"], &exec, &|_| ApprovalDecision::Run, false);
        assert_eq!(tool_turn(&eng), "Error: ReferenceError: x is not defined");
    }

    #[test]
    fn run_js_without_code_tools_never_executes() {
        // The model names run_js although it wasn't offered: unknown tool.
        let eng = engine(vec![JS_CALL, "no"]);
        let rec = Arc::new(Recorder::default());
        let sink: Arc<dyn ChatSink> = rec.clone();
        let dyn_eng: Arc<dyn Engine> = eng.clone();
        let out = run(&dyn_eng, "h", request(), &fake_registry(), &sink, &|| false, None).unwrap();
        assert!(out.steps.is_empty());
        assert!(tool_turn(&eng).starts_with("Error"));
    }

    #[test]
    fn code_runs_obey_the_round_cap() {
        let exec = MockExecutor::new();
        let (out, eng, _) = run_with_js(vec![JS_CALL], &exec, &|_| ApprovalDecision::Run, false);
        assert_eq!(eng.calls.load(Ordering::SeqCst), MAX_ROUNDS);
        assert_eq!(exec.runs.lock().unwrap().len(), MAX_ROUNDS - 1);
        assert!(out.text.contains("stopped"));
    }

    #[test]
    fn specs_offer_only_what_is_enabled() {
        let names = |w, j, p| -> Vec<String> {
            tool_specs(w, j, p, false).into_iter().map(|t| t.name).collect()
        };
        assert!(!names(true, false, false).contains(&"run_js".to_string()));
        assert!(!names(false, true, false).contains(&"web_search".to_string()));
        assert!(names(false, true, false).contains(&"run_js".to_string()));
        assert!(!names(false, true, false).contains(&"run_python".to_string()));
        assert!(names(false, false, true).contains(&"run_python".to_string()));
        assert!(!names(false, false, true).contains(&"run_js".to_string()));
    }

    // ---- AGT-13 stage 2: run_on_mac ----

    fn mac_call(lang: &str, code: &str) -> &'static str {
        Box::leak(
            format!(
                "<tool_call>{{\"name\": \"run_on_mac\", \"arguments\": {{\"language\": \"{lang}\", \"code\": \"{code}\"}}}}</tool_call>"
            )
            .into_boxed_str(),
        )
    }

    #[test]
    fn run_on_mac_always_asks_even_with_always_allow_and_routes_to_the_mac() {
        let exec = MockExecutor::new();
        let asked = Mutex::new(vec![]);
        let (out, eng, rec) = run_with(
            vec![mac_call("shell", "uname -a"), "Done."],
            &exec,
            &|r| {
                asked.lock().unwrap().push(r);
                ApprovalDecision::RunEdited { code: "uname -m".into() }
            },
            true, // JS "always allow" must never skip the Mac card
            (true, false, true),
        );
        assert_eq!(out.text, "Done.");
        let asked = asked.lock().unwrap();
        assert_eq!(asked.len(), 1);
        assert_eq!((asked[0].tool.as_str(), asked[0].summary.as_str()), ("run_on_mac", "Run Shell on your Mac"));
        assert_eq!(
            *exec.runs.lock().unwrap(),
            vec![("mac:shell".to_string(), "uname -m".to_string(), 10_000)]
        );
        assert_eq!(tool_turn(&eng), "42");
        let step = rec.steps.lock().unwrap()[0].clone();
        assert_eq!(step.label, "mac:shell");
        assert!(step.detail.contains("\"target\":\"mac\""));
    }

    #[test]
    fn denied_or_badly_specified_mac_runs_never_execute() {
        let exec = MockExecutor::new();
        let (_, eng, _) = run_with(vec![mac_call("shell", "ls"), "ok"], &exec, &|_| ApprovalDecision::Deny, false, (false, false, true));
        assert!(exec.runs.lock().unwrap().is_empty());
        assert_eq!(tool_turn(&eng), "The user chose not to run this.");
        let (_, eng, _) = run_with(vec![mac_call("ruby", "puts 1"), "ok"], &exec, &|_| panic!("must not ask"), false, (false, false, true));
        assert!(exec.runs.lock().unwrap().is_empty());
        assert!(tool_turn(&eng).starts_with("Error: run_on_mac needs language"));
    }

    #[test]
    fn run_on_mac_is_unknown_unless_offered() {
        let exec = MockExecutor::new();
        let (_, eng, _) = run_with(vec![mac_call("shell", "ls"), "ok"], &exec, &|_| ApprovalDecision::Run, false, (true, true, false));
        assert!(exec.runs.lock().unwrap().is_empty());
        assert!(tool_turn(&eng).starts_with("Error"));
        let names = |mac| -> Vec<String> {
            tool_specs(false, false, false, mac).into_iter().map(|t| t.name).collect()
        };
        assert!(names(true).contains(&"run_on_mac".to_string()));
        assert!(!names(false).contains(&"run_on_mac".to_string()));
        assert!(code_prompt(false, false, true).contains("run_on_mac"));
        assert!(!code_prompt(true, false, false).contains("Mac"));
    }

    // ---- AGT-13 stage 3: run_python ----

    const PY_CALL: &str =
        "<tool_call>{\"name\": \"run_python\", \"arguments\": {\"code\": \"print(6*7)\"}}</tool_call>";

    #[test]
    fn python_asks_every_time_even_when_js_is_always_allowed() {
        let exec = MockExecutor::new();
        let asked = Mutex::new(vec![]);
        let (out, eng, rec) = run_with(
            vec![PY_CALL, "It is 42."],
            &exec,
            &|r| {
                asked.lock().unwrap().push(r);
                ApprovalDecision::Run
            },
            true, // the chat's JS "always allow" must not skip Python's card
            (true, true, false),
        );
        assert_eq!(out.text, "It is 42.");
        let asked = asked.lock().unwrap();
        assert_eq!((asked.len(), asked[0].tool.as_str(), asked[0].code.as_str()), (1, "run_python", "print(6*7)"));
        assert_eq!(
            *exec.runs.lock().unwrap(),
            vec![("python".to_string(), "print(6*7)".to_string(), 10_000)]
        );
        assert_eq!(tool_turn(&eng), "42");
        assert_eq!(rec.steps.lock().unwrap()[0].label, "python");
    }

    #[test]
    fn denied_python_never_runs() {
        let exec = MockExecutor::new();
        let (_, eng, rec) =
            run_with(vec![PY_CALL, "ok"], &exec, &|_| ApprovalDecision::Deny, false, (false, true, false));
        assert!(exec.runs.lock().unwrap().is_empty());
        assert_eq!(tool_turn(&eng), "The user chose not to run this.");
        assert!(rec.steps.lock().unwrap()[0].detail.contains("\"denied\":true"));
    }

    #[test]
    fn a_python_timeout_reaches_the_model_as_an_error() {
        let mut exec = MockExecutor::new();
        exec.result = ToolRunResult {
            ok: false,
            output: String::new(),
            error: "ran too long (10 s)".into(),
            duration_ms: 10_000,
            truncated: false,
        };
        let (_, eng, rec) = run_with(vec![PY_CALL, "sorry"], &exec, &|_| ApprovalDecision::Run, false, (false, true, false));
        assert_eq!(tool_turn(&eng), "Error: ran too long (10 s)");
        assert!(rec.steps.lock().unwrap()[0].detail.contains("ran too long"));
    }

    #[test]
    fn a_tool_that_was_not_offered_never_executes() {
        // JS only on: run_python is an unknown tool, and vice versa.
        let exec = MockExecutor::new();
        let (_, eng, _) = run_with(vec![PY_CALL, "no"], &exec, &|_| panic!("no card"), false, (true, false, false));
        assert!(exec.runs.lock().unwrap().is_empty());
        assert!(tool_turn(&eng).starts_with("Error"));
        let (_, _, _) = run_with(vec![JS_CALL, "no"], &exec, &|_| panic!("no card"), false, (false, true, false));
        assert!(exec.runs.lock().unwrap().is_empty());
    }

    #[test]
    fn the_code_prompt_names_only_offered_languages() {
        assert!(!code_prompt(true, false, false).contains("Python"));
        assert!(!code_prompt(false, true, false).contains("JavaScript"));
        let both = code_prompt(true, true, false);
        assert!(both.contains("run_js") && both.contains("run_python"));
    }

    #[test]
    fn an_unanswered_approval_times_out_to_no() {
        let (_tx, rx) = std::sync::mpsc::channel::<ApprovalDecision>();
        let t = std::time::Instant::now();
        let d = await_decision(&rx, std::time::Duration::from_millis(150), &|| false);
        assert_eq!(d, ApprovalDecision::Deny);
        assert!(t.elapsed() >= std::time::Duration::from_millis(150));
    }

    #[test]
    fn stop_ends_the_approval_wait() {
        let (_tx, rx) = std::sync::mpsc::channel::<ApprovalDecision>();
        let d = await_decision(&rx, std::time::Duration::from_secs(60), &|| true);
        assert_eq!(d, ApprovalDecision::Deny);
    }

    #[test]
    fn a_decision_sent_while_waiting_is_returned() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(ApprovalDecision::Run).unwrap();
        assert_eq!(
            await_decision(&rx, std::time::Duration::from_secs(5), &|| false),
            ApprovalDecision::Run
        );
    }

    /// Hits DuckDuckGo for real: `cargo test -p pocket-ffi live_search -- --ignored`.
    #[test]
    #[ignore]
    fn live_search_smoke() {
        let egress = Arc::new(Mutex::new(vec![]));
        let reg = registry(|| 0, http_fetcher(egress.clone(), || 0));
        let out = reg
            .call("web_search", r#"{"query": "latest macOS version"}"#)
            .unwrap();
        assert!(out.contains("http"), "{out}");
        assert_eq!(egress.lock().unwrap().len(), 1);
    }
}
