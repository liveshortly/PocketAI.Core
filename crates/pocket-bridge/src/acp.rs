//! ACP (Agent Client Protocol) client — AGT-06.
//!
//! JSON-RPC 2.0 over newline-delimited JSON on a child process's stdio.
//! Client side only: `initialize`, `session/new`, `session/prompt`,
//! `session/cancel`; incoming `session/update` notifications become
//! [`AcpEvent`]s on an mpsc channel, and incoming `session/request_permission`
//! requests become [`pocket_agents::Approval`]s routed through the embedder's
//! [`PermissionHandler`].
//!
//! Threads + channels, no async. One reader thread per client dispatches
//! every incoming line; outgoing requests block the calling thread on a
//! per-request channel until the matching response arrives.
//!
//! Wire shapes follow the public ACP spec (agentclientprotocol.com).
//! // TODO(AGT-09): verify field names against pinned adapters.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use pocket_agents::{risk, Approval, ApprovalKind, Decision};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum AcpError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("agent error {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("transport closed")]
    Closed,
}

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

/// One line out, one line in. Implementations are shared between the caller
/// thread (send) and the reader thread (recv), hence `&self` + Send + Sync.
pub trait Transport: Send + Sync {
    fn send(&self, line: &str) -> io::Result<()>;
    /// Blocks for the next line; `Ok(None)` on EOF.
    fn recv(&self) -> io::Result<Option<String>>;
}

/// Real transport: an agent child process speaking ACP on stdio.
///
/// The child runs in its own process group (`process_group(0)`, i.e. setpgid —
/// stdlib, no libc dep) so AGT-10 can signal the whole group. stderr goes to
/// the session log file supplied by the caller.
pub struct ChildTransport {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    stdout: Mutex<BufReader<ChildStdout>>,
    pid: u32,
}

impl ChildTransport {
    pub fn spawn(
        program: &str,
        args: &[&str],
        cwd: Option<&Path>,
        stderr_log: &Path,
    ) -> io::Result<Self> {
        let log = std::fs::File::create(stderr_log)?;
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(log));
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        Ok(Self {
            pid: child.id(),
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            stdout: Mutex::new(BufReader::new(stdout)),
        })
    }

    /// Child pid (= process group id, thanks to `process_group(0)`).
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Kill the child process. Best-effort, idempotent.
    // ponytail: kills the direct child only; group-wide SIGINT→SIGKILL
    // escalation lands in AGT-10 using -pid.
    pub fn kill(&self) {
        let mut child = self.child.lock().unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl Drop for ChildTransport {
    fn drop(&mut self) {
        self.kill();
    }
}

impl Transport for ChildTransport {
    fn send(&self, line: &str) -> io::Result<()> {
        let mut stdin = self.stdin.lock().unwrap();
        stdin.write_all(line.as_bytes())?;
        stdin.write_all(b"\n")?;
        stdin.flush()
    }

    fn recv(&self) -> io::Result<Option<String>> {
        let mut stdout = self.stdout.lock().unwrap();
        let mut line = String::new();
        match stdout.read_line(&mut line)? {
            0 => Ok(None),
            _ => Ok(Some(line.trim_end_matches(['\r', '\n']).to_string())),
        }
    }
}

// ---------------------------------------------------------------------------
// Events and permissions
// ---------------------------------------------------------------------------

/// Typed stream of `session/update` notifications.
#[derive(Clone, Debug, PartialEq)]
pub enum AcpEvent {
    /// `agent_message_chunk`
    MessageChunk { session_id: String, text: String },
    /// `agent_thought_chunk`
    ThoughtChunk { session_id: String, text: String },
    /// `tool_call`
    ToolCall {
        session_id: String,
        tool_call_id: String,
        title: String,
        kind: String,
        status: String,
        raw: Value,
    },
    /// `tool_call_update`
    ToolCallUpdate {
        session_id: String,
        tool_call_id: String,
        status: Option<String>,
        raw: Value,
    },
    /// `plan`
    Plan { session_id: String, entries: Value },
    /// Update kinds this client does not model yet; raw payload passed through.
    Other { session_id: String, raw: Value },
}

/// Supplied by the embedder. Called on the reader thread with a freshly
/// classified, pending [`Approval`]; blocks until a decision exists (the
/// bridge's approval machinery + timeout live behind this). The returned
/// [`Decision`] is mapped to the closest ACP permission option id.
pub trait PermissionHandler: Send + Sync {
    fn request(&self, approval: Approval) -> Decision;
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// In-flight requests by id. `None` once the reader has hit EOF: closing and
/// registering happen under one lock, so a request can never slip in after the
/// final drain and wait forever.
type Pending = Arc<Mutex<Option<HashMap<i64, mpsc::Sender<Result<Value, AcpError>>>>>>;
type Cwds = Arc<Mutex<HashMap<String, PathBuf>>>;

pub struct AcpClient {
    transport: Arc<dyn Transport>,
    next_id: AtomicI64,
    pending: Pending,
    session_cwds: Cwds,
    _reader: thread::JoinHandle<()>,
}

impl AcpClient {
    /// Starts the reader thread. Returns the client and the event stream.
    pub fn new(
        transport: Arc<dyn Transport>,
        handler: Arc<dyn PermissionHandler>,
    ) -> (Self, mpsc::Receiver<AcpEvent>) {
        let (event_tx, event_rx) = mpsc::channel();
        let pending: Pending = Arc::new(Mutex::new(Some(HashMap::new())));
        let session_cwds: Cwds = Arc::default();
        let reader = {
            let transport = Arc::clone(&transport);
            let pending = Arc::clone(&pending);
            let session_cwds = Arc::clone(&session_cwds);
            thread::spawn(move || {
                reader_loop(&*transport, &pending, &session_cwds, &*handler, &event_tx)
            })
        };
        (
            Self {
                transport,
                next_id: AtomicI64::new(1),
                pending,
                session_cwds,
                _reader: reader,
            },
            event_rx,
        )
    }

    /// ACP `initialize`. fs read/write capabilities are OFF: the agent uses
    /// its own tools, the bridge only gates them via permissions.
    pub fn initialize(&self) -> Result<Value, AcpError> {
        self.request(
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {
                    "fs": { "readTextFile": false, "writeTextFile": false }
                }
            }),
        )
    }

    /// ACP `session/new` → session id. Remembers `cwd` as the repo root for
    /// risk classification of this session's permission requests.
    pub fn session_new(&self, cwd: &Path) -> Result<String, AcpError> {
        let result = self.request(
            "session/new",
            json!({ "cwd": cwd.to_string_lossy(), "mcpServers": [] }),
        )?;
        let session_id = result["sessionId"]
            .as_str()
            .ok_or_else(|| AcpError::Protocol(format!("session/new missing sessionId: {result}")))?
            .to_string();
        self.session_cwds
            .lock()
            .unwrap()
            .insert(session_id.clone(), cwd.to_path_buf());
        Ok(session_id)
    }

    /// ACP `session/prompt`. Blocks until the turn ends; updates stream on the
    /// event channel meanwhile. Returns the stop reason (e.g. "end_turn").
    pub fn prompt(&self, session_id: &str, text: &str) -> Result<String, AcpError> {
        let result = self.request(
            "session/prompt",
            json!({
                "sessionId": session_id,
                "prompt": [ { "type": "text", "text": text } ]
            }),
        )?;
        Ok(result["stopReason"].as_str().unwrap_or("unknown").to_string())
    }

    /// ACP `session/cancel` notification (no response).
    pub fn cancel(&self, session_id: &str) -> Result<(), AcpError> {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": "session/cancel",
            "params": { "sessionId": session_id }
        });
        self.transport.send(&msg.to_string())?;
        Ok(())
    }

    fn request(&self, method: &str, params: Value) -> Result<Value, AcpError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        self.pending
            .lock()
            .unwrap()
            .as_mut()
            .ok_or(AcpError::Closed)?
            .insert(id, tx);
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if let Err(e) = self.transport.send(&msg.to_string()) {
            if let Some(map) = self.pending.lock().unwrap().as_mut() {
                map.remove(&id);
            }
            return Err(e.into());
        }
        rx.recv().map_err(|_| AcpError::Closed)?
    }
}

// ---------------------------------------------------------------------------
// Reader thread
// ---------------------------------------------------------------------------

fn reader_loop(
    transport: &dyn Transport,
    pending: &Pending,
    session_cwds: &Cwds,
    handler: &dyn PermissionHandler,
    events: &mpsc::Sender<AcpEvent>,
) {
    loop {
        let line = match transport.recv() {
            Ok(Some(line)) => line,
            Ok(None) | Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue; // not JSON: ignore (adapters sometimes log to stdout)
        };
        if msg.get("method").is_none() {
            route_response(pending, &msg);
        } else if let Some(id) = msg.get("id").cloned() {
            handle_agent_request(transport, session_cwds, handler, id, &msg);
        } else if msg["method"] == "session/update" {
            if let Some(event) = parse_update(&msg["params"]) {
                let _ = events.send(event);
            }
        }
        // other notifications: ignored
    }
    // EOF / transport error: fail every in-flight request.
    // Taking the map also closes it to new requests.
    let inflight = pending.lock().unwrap().take().unwrap_or_default();
    for (_, tx) in inflight {
        let _ = tx.send(Err(AcpError::Closed));
    }
}

fn route_response(pending: &Pending, msg: &Value) {
    let Some(id) = msg["id"].as_i64() else { return };
    let Some(tx) = pending.lock().unwrap().as_mut().and_then(|m| m.remove(&id)) else {
        return;
    };
    let result = if let Some(err) = msg.get("error") {
        Err(AcpError::Rpc {
            code: err["code"].as_i64().unwrap_or(0),
            message: err["message"].as_str().unwrap_or("").to_string(),
        })
    } else {
        Ok(msg["result"].clone())
    };
    let _ = tx.send(result);
}

/// Agent → client requests. Only `session/request_permission` is supported;
/// everything else gets a JSON-RPC "method not found".
// ponytail: permission handled inline on the reader thread — updates queue
// behind the decision, but the agent is blocked on this reply anyway. Move to
// a per-request thread if adapters turn out to interleave.
fn handle_agent_request(
    transport: &dyn Transport,
    session_cwds: &Cwds,
    handler: &dyn PermissionHandler,
    id: Value,
    msg: &Value,
) {
    let reply = if msg["method"] == "session/request_permission" {
        let outcome = permission_outcome(session_cwds, handler, &msg["params"]);
        json!({ "jsonrpc": "2.0", "id": id, "result": { "outcome": outcome } })
    } else {
        json!({
            "jsonrpc": "2.0", "id": id,
            "error": { "code": -32601, "message": format!("method not found: {}", msg["method"]) }
        })
    };
    let _ = transport.send(&reply.to_string());
}

/// Build an [`Approval`] from the permission request (risk via
/// `pocket_agents::risk::classify`), block on the embedder's decision, and
/// map it to one of the offered option ids.
fn permission_outcome(session_cwds: &Cwds, handler: &dyn PermissionHandler, params: &Value) -> Value {
    let session_id = params["sessionId"].as_str().unwrap_or_default();
    let tool_call = &params["toolCall"];
    let (kind, detail) = map_tool_call(tool_call);
    let cwd = session_cwds.lock().unwrap().get(session_id).cloned();
    let risk = risk::classify(&kind, &detail, cwd.as_deref());
    let title = tool_call["title"].as_str().unwrap_or("Agent action");
    let approval = Approval::new(
        session_id,
        kind,
        risk,
        title,
        tool_call.to_string(),
        pocket_agents::now_ms(),
    );
    let decision = handler.request(approval);
    match pick_option(decision, &params["options"]) {
        Some(option_id) => json!({ "outcome": "selected", "optionId": option_id }),
        // No matching option offered (or none at all): cancel the request,
        // which every ACP agent must treat as "not allowed".
        None => json!({ "outcome": "cancelled" }),
    }
}

/// ACP tool-call kind → ApprovalKind + the detail string `risk::classify`
/// wants (command line for commands, target path for file ops).
/// // TODO(AGT-09): verify kind strings and rawInput shapes against pinned adapters.
fn map_tool_call(tool_call: &Value) -> (ApprovalKind, String) {
    let raw = &tool_call["rawInput"];
    let title = tool_call["title"].as_str().unwrap_or_default();
    let kind = match tool_call["kind"].as_str().unwrap_or("") {
        "execute" => ApprovalKind::Command,
        "edit" => ApprovalKind::FileEdit,
        "delete" => ApprovalKind::FileDelete,
        "fetch" => ApprovalKind::Network,
        _ => ApprovalKind::Other, // read/search/think/move/unknown
    };
    let detail = match kind {
        ApprovalKind::Command => raw["command"].as_str().unwrap_or(title).to_string(),
        ApprovalKind::FileEdit | ApprovalKind::FileCreate | ApprovalKind::FileDelete => raw
            ["path"]
            .as_str()
            .or_else(|| tool_call["locations"][0]["path"].as_str())
            .unwrap_or(title)
            .to_string(),
        _ => title.to_string(),
    };
    (kind, detail)
}

/// Map our Decision onto the agent's offered permission options by their
/// `kind` ("allow_once" | "allow_always" | "reject_once" | "reject_always"),
/// preferring the exact match, falling back to the same polarity.
fn pick_option(decision: Decision, options: &Value) -> Option<String> {
    let prefs: &[&str] = match decision {
        Decision::ApproveOnce => &["allow_once", "allow_always"],
        Decision::ApproveSession => &["allow_always", "allow_once"],
        Decision::Deny => &["reject_once", "reject_always"],
        Decision::DenyAndStop => &["reject_always", "reject_once"],
    };
    let options = options.as_array()?;
    for want in prefs {
        if let Some(opt) = options.iter().find(|o| o["kind"] == *want) {
            return opt["optionId"].as_str().map(String::from);
        }
    }
    None
}

fn parse_update(params: &Value) -> Option<AcpEvent> {
    let session_id = params["sessionId"].as_str().unwrap_or_default().to_string();
    let update = &params["update"];
    let event = match update["sessionUpdate"].as_str()? {
        "agent_message_chunk" => AcpEvent::MessageChunk {
            session_id,
            text: update["content"]["text"].as_str().unwrap_or_default().to_string(),
        },
        "agent_thought_chunk" => AcpEvent::ThoughtChunk {
            session_id,
            text: update["content"]["text"].as_str().unwrap_or_default().to_string(),
        },
        "tool_call" => AcpEvent::ToolCall {
            session_id,
            tool_call_id: update["toolCallId"].as_str().unwrap_or_default().to_string(),
            title: update["title"].as_str().unwrap_or_default().to_string(),
            kind: update["kind"].as_str().unwrap_or_default().to_string(),
            status: update["status"].as_str().unwrap_or_default().to_string(),
            raw: update.clone(),
        },
        "tool_call_update" => AcpEvent::ToolCallUpdate {
            session_id,
            tool_call_id: update["toolCallId"].as_str().unwrap_or_default().to_string(),
            status: update["status"].as_str().map(String::from),
            raw: update.clone(),
        },
        "plan" => AcpEvent::Plan {
            session_id,
            entries: update["entries"].clone(),
        },
        _ => AcpEvent::Other {
            session_id,
            raw: update.clone(),
        },
    };
    Some(event)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use pocket_agents::Risk;

    /// Mock transport: `send` lands on a channel the test drains, `recv`
    /// blocks on a channel the test (or fake agent thread) feeds.
    struct MockTransport {
        sent: mpsc::Sender<String>,
        incoming: Mutex<mpsc::Receiver<String>>,
    }

    impl Transport for MockTransport {
        fn send(&self, line: &str) -> io::Result<()> {
            self.sent
                .send(line.to_string())
                .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "mock closed"))
        }
        fn recv(&self) -> io::Result<Option<String>> {
            Ok(self.incoming.lock().unwrap().recv().ok())
        }
    }

    /// Permission handler returning a fixed decision, recording each approval.
    struct FixedHandler {
        decision: Decision,
        seen: Mutex<Vec<Approval>>,
    }

    impl FixedHandler {
        fn new(decision: Decision) -> Arc<Self> {
            Arc::new(Self {
                decision,
                seen: Mutex::new(Vec::new()),
            })
        }
    }

    impl PermissionHandler for FixedHandler {
        fn request(&self, approval: Approval) -> Decision {
            self.seen.lock().unwrap().push(approval);
            self.decision
        }
    }

    /// Scripted fake agent: a thread that reads the client's outgoing lines
    /// and answers via `script(request_json) -> Vec<reply lines>`.
    fn with_fake_agent<F>(
        handler: Arc<dyn PermissionHandler>,
        script: F,
    ) -> (AcpClient, mpsc::Receiver<AcpEvent>)
    where
        F: Fn(&Value) -> Vec<Value> + Send + 'static,
    {
        let (sent_tx, sent_rx) = mpsc::channel::<String>();
        let (in_tx, in_rx) = mpsc::channel::<String>();
        thread::spawn(move || {
            while let Ok(line) = sent_rx.recv() {
                let msg: Value = serde_json::from_str(&line).unwrap();
                for reply in script(&msg) {
                    if in_tx.send(reply.to_string()).is_err() {
                        return;
                    }
                }
            }
        });
        let transport = Arc::new(MockTransport {
            sent: sent_tx,
            incoming: Mutex::new(in_rx),
        });
        AcpClient::new(transport, handler)
    }

    fn respond(req: &Value, result: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": req["id"], "result": result })
    }

    #[test]
    fn initialize_handshake_sends_fs_off_and_returns_result() {
        let handler = FixedHandler::new(Decision::Deny);
        let (client, _events) = with_fake_agent(handler, |req| {
            assert_eq!(req["method"], "initialize");
            assert_eq!(req["params"]["protocolVersion"], 1);
            // fs capabilities must be OFF
            assert_eq!(req["params"]["clientCapabilities"]["fs"]["readTextFile"], false);
            assert_eq!(req["params"]["clientCapabilities"]["fs"]["writeTextFile"], false);
            vec![respond(req, json!({ "protocolVersion": 1 }))]
        });
        let result = client.initialize().unwrap();
        assert_eq!(result["protocolVersion"], 1);
    }

    #[test]
    fn session_new_sends_cwd_and_empty_mcp_servers() {
        let handler = FixedHandler::new(Decision::Deny);
        let (client, _events) = with_fake_agent(handler, |req| match req["method"].as_str() {
            Some("session/new") => {
                assert_eq!(req["params"]["cwd"], "/tmp/repo");
                assert_eq!(req["params"]["mcpServers"], json!([]));
                vec![respond(req, json!({ "sessionId": "sess-123" }))]
            }
            other => panic!("unexpected method {other:?}"),
        });
        let id = client.session_new(Path::new("/tmp/repo")).unwrap();
        assert_eq!(id, "sess-123");
    }

    #[test]
    fn prompt_streams_updates_then_returns_stop_reason() {
        let handler = FixedHandler::new(Decision::Deny);
        let (client, events) = with_fake_agent(handler, |req| match req["method"].as_str() {
            Some("session/new") => vec![respond(req, json!({ "sessionId": "s1" }))],
            Some("session/prompt") => {
                assert_eq!(req["params"]["sessionId"], "s1");
                assert_eq!(req["params"]["prompt"][0]["text"], "list files");
                let update = |u: Value| {
                    json!({
                        "jsonrpc": "2.0", "method": "session/update",
                        "params": { "sessionId": "s1", "update": u }
                    })
                };
                vec![
                    update(json!({
                        "sessionUpdate": "agent_message_chunk",
                        "content": { "type": "text", "text": "Hello" }
                    })),
                    update(json!({
                        "sessionUpdate": "tool_call",
                        "toolCallId": "tc1", "title": "ls", "kind": "execute",
                        "status": "pending"
                    })),
                    update(json!({
                        "sessionUpdate": "tool_call_update",
                        "toolCallId": "tc1", "status": "completed"
                    })),
                    update(json!({
                        "sessionUpdate": "plan",
                        "entries": [ { "content": "step 1", "status": "pending" } ]
                    })),
                    respond(req, json!({ "stopReason": "end_turn" })),
                ]
            }
            other => panic!("unexpected method {other:?}"),
        });

        let sid = client.session_new(Path::new("/tmp/repo")).unwrap();
        let stop = client.prompt(&sid, "list files").unwrap();
        assert_eq!(stop, "end_turn");

        assert_eq!(
            events.recv().unwrap(),
            AcpEvent::MessageChunk { session_id: "s1".into(), text: "Hello".into() }
        );
        match events.recv().unwrap() {
            AcpEvent::ToolCall { tool_call_id, title, kind, status, .. } => {
                assert_eq!(tool_call_id, "tc1");
                assert_eq!(title, "ls");
                assert_eq!(kind, "execute");
                assert_eq!(status, "pending");
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
        match events.recv().unwrap() {
            AcpEvent::ToolCallUpdate { tool_call_id, status, .. } => {
                assert_eq!(tool_call_id, "tc1");
                assert_eq!(status.as_deref(), Some("completed"));
            }
            other => panic!("expected ToolCallUpdate, got {other:?}"),
        }
        match events.recv().unwrap() {
            AcpEvent::Plan { entries, .. } => assert_eq!(entries[0]["content"], "step 1"),
            other => panic!("expected Plan, got {other:?}"),
        }
    }

    #[test]
    fn request_permission_creates_classified_approval_and_replies_option_id() {
        let handler = FixedHandler::new(Decision::ApproveOnce);
        let h = Arc::clone(&handler);
        let (client, _events) = with_fake_agent(h, |req| match req["method"].as_str() {
            Some("session/new") => vec![respond(req, json!({ "sessionId": "s1" }))],
            Some("session/prompt") => vec![json!({
                "jsonrpc": "2.0", "id": 777, "method": "session/request_permission",
                "params": {
                    "sessionId": "s1",
                    "toolCall": {
                        "toolCallId": "tc1", "title": "Run git push", "kind": "execute",
                        "rawInput": { "command": "git push origin main" }
                    },
                    "options": [
                        { "optionId": "allow", "name": "Allow", "kind": "allow_once" },
                        { "optionId": "allow-always", "name": "Always", "kind": "allow_always" },
                        { "optionId": "deny", "name": "Deny", "kind": "reject_once" }
                    ]
                }
            })],
            None => {
                // The client's reply to the permission request (a response,
                // not a request): check it, then end the turn.
                assert_eq!(req["id"], 777);
                assert_eq!(req["result"]["outcome"]["outcome"], "selected");
                assert_eq!(req["result"]["outcome"]["optionId"], "allow");
                vec![json!({ "jsonrpc": "2.0", "id": 2, "result": { "stopReason": "end_turn" } })]
            }
            other => panic!("unexpected method {other:?}"),
        });

        let sid = client.session_new(Path::new("/tmp/repo")).unwrap();
        let stop = client.prompt(&sid, "push it").unwrap();
        assert_eq!(stop, "end_turn");

        let seen = handler.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        let approval = &seen[0];
        assert_eq!(approval.session_id, "s1");
        assert_eq!(approval.kind, ApprovalKind::Command);
        assert_eq!(approval.risk, Risk::High, "git push classifies High");
        assert_eq!(approval.summary, "Run git push");
        assert!(approval.detail_json.contains("git push origin main"));
    }

    #[test]
    fn deny_maps_to_reject_option_and_no_match_cancels() {
        // Deny → reject_once option id.
        assert_eq!(
            pick_option(
                Decision::Deny,
                &json!([
                    { "optionId": "a", "kind": "allow_once" },
                    { "optionId": "r", "kind": "reject_once" }
                ])
            ),
            Some("r".to_string())
        );
        // DenyAndStop falls back to reject_once when reject_always is absent.
        assert_eq!(
            pick_option(
                Decision::DenyAndStop,
                &json!([{ "optionId": "r", "kind": "reject_once" }])
            ),
            Some("r".to_string())
        );
        // ApproveSession prefers allow_always.
        assert_eq!(
            pick_option(
                Decision::ApproveSession,
                &json!([
                    { "optionId": "a1", "kind": "allow_once" },
                    { "optionId": "aa", "kind": "allow_always" }
                ])
            ),
            Some("aa".to_string())
        );
        // No matching polarity → None → cancelled outcome.
        assert_eq!(
            pick_option(Decision::Deny, &json!([{ "optionId": "a", "kind": "allow_once" }])),
            None
        );
    }

    #[test]
    fn cancel_sends_notification_without_id() {
        let (sent_tx, sent_rx) = mpsc::channel::<String>();
        let (_in_tx, in_rx) = mpsc::channel::<String>();
        let transport = Arc::new(MockTransport {
            sent: sent_tx,
            incoming: Mutex::new(in_rx),
        });
        let (client, _events) = AcpClient::new(transport, FixedHandler::new(Decision::Deny));
        client.cancel("s1").unwrap();
        let line: Value = serde_json::from_str(&sent_rx.recv().unwrap()).unwrap();
        assert_eq!(line["method"], "session/cancel");
        assert_eq!(line["params"]["sessionId"], "s1");
        assert!(line.get("id").is_none(), "notifications carry no id");
    }

    #[test]
    fn unknown_agent_request_gets_method_not_found() {
        let handler = FixedHandler::new(Decision::Deny);
        let (client, _events) = with_fake_agent(handler, |req| match req["method"].as_str() {
            Some("initialize") => vec![
                json!({
                    "jsonrpc": "2.0", "id": 99, "method": "fs/read_text_file",
                    "params": { "path": "/etc/passwd" }
                }),
                respond(req, json!({ "protocolVersion": 1 })),
            ],
            None => {
                assert_eq!(req["id"], 99);
                assert_eq!(req["error"]["code"], -32601);
                vec![]
            }
            other => panic!("unexpected method {other:?}"),
        });
        client.initialize().unwrap();
    }

    #[test]
    fn transport_eof_fails_inflight_requests() {
        let (sent_tx, _sent_rx) = mpsc::channel::<String>();
        let (in_tx, in_rx) = mpsc::channel::<String>();
        let transport = Arc::new(MockTransport {
            sent: sent_tx,
            incoming: Mutex::new(in_rx),
        });
        let (client, _events) = AcpClient::new(transport, FixedHandler::new(Decision::Deny));
        drop(in_tx); // EOF
        assert!(matches!(client.initialize(), Err(AcpError::Closed)));
    }

    // -- ChildTransport ------------------------------------------------------

    #[test]
    fn child_transport_cat_echoes_lines() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("stderr.log");
        let t = ChildTransport::spawn("/bin/cat", &[], None, &log).unwrap();
        t.send(r#"{"ping":1}"#).unwrap();
        assert_eq!(t.recv().unwrap().as_deref(), Some(r#"{"ping":1}"#));
        t.send("second").unwrap();
        assert_eq!(t.recv().unwrap().as_deref(), Some("second"));
        t.kill();
        assert_eq!(t.recv().unwrap(), None, "EOF after kill");
        assert!(log.exists(), "stderr log file created");
    }

    #[test]
    fn child_transport_sh_answers_initialize_through_client() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("stderr.log");
        // Deterministic one-shot agent: answer the first request (id 1 — the
        // client's ids start at 1) with an initialize result, log to stderr.
        let script = r#"read line; echo "answering" >&2; printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}'"#;
        let transport =
            Arc::new(ChildTransport::spawn("/bin/sh", &["-c", script], None, &log).unwrap());
        let (client, _events) = AcpClient::new(transport, FixedHandler::new(Decision::Deny));
        let result = client.initialize().unwrap();
        assert_eq!(result["protocolVersion"], 1);
        assert_eq!(std::fs::read_to_string(&log).unwrap().trim(), "answering");
    }

    // -- Real-agent integration skeleton (AGT-09 fills this in) --------------

    /// Requires OpenCode installed and `--features agent-tests`. Skeleton only.
    #[cfg(feature = "agent-tests")]
    #[test]
    #[ignore = "requires `opencode` on PATH; run with --features agent-tests -- --ignored"]
    fn opencode_initialize_and_session_new() {
        let repo = tempfile::tempdir().unwrap();
        let log = repo.path().join("opencode-stderr.log");
        let transport = Arc::new(
            ChildTransport::spawn("opencode", &["acp"], Some(repo.path()), &log)
                .expect("opencode not installed"),
        );
        let (client, _events) = AcpClient::new(transport, FixedHandler::new(Decision::Deny));
        client.initialize().expect("initialize");
        let sid = client.session_new(repo.path()).expect("session/new");
        assert!(!sid.is_empty());
        client.cancel(&sid).expect("session/cancel");
        // TODO(AGT-09): prompt "list files in this repo" and assert updates.
    }
}
