//! NET-13: approvals, agents, and kill endpoints, delegating every state
//! transition to `pocket-agents` (`Approvals`, the AGT-02 state machine).
//!
//! The referenced endpoint spec (08-agents-spec.md §5.2) does not exist yet;
//! this module defines the minimal REST surface:
//!
//! - `GET  /pocket/v1/approvals` — pending approvals, each in the shared
//!   approval payload shape of `agents.md` §5 (`id`, `session_id`, `agent`,
//!   `repo`, `kind`, `risk`, `title`, `command`, `files`, `reason`,
//!   `options`, `created_at`, `expires_at`). File diffs over 400 lines are
//!   truncated with `truncated: true`. Overdue approvals are expired (swept)
//!   before listing.
//! - `GET  /pocket/v1/approvals/{id}` — one approval in any state: the same
//!   payload plus `state` and `decision`.
//! - `POST /pocket/v1/approvals/{id}` — body `{decision, idempotency_key,
//!   rule?}`. `idempotency_key` is required (400 without). A duplicate
//!   decision returns the FIRST recorded outcome (200, same body); the state
//!   machine guarantees no second transition. Reusing the settling key with
//!   a different decision is a client bug → **409** `idempotency_key_reused`.
//!   An expired approval returns **409** with the expired outcome in the
//!   body (matches agents.md §7: "iPhone decides after expiry → 409").
//!   `approve_session` needs `rule` (400 without) and is rejected for
//!   high-risk approvals (400, no transition).
//! - `GET  /pocket/v1/agents` — agent sessions with their status.
//! - `POST /pocket/v1/agents/{id}/kill` — stop one session: status →
//!   `stopped`, its pending approvals cancelled.
//! - `POST /pocket/v1/kill` — kill switch: all sessions stopped, every
//!   pending approval denied, session rules disabled (agents.md §7).
//!
//! Errors use the lean `/pocket/v1` shape (`pocket_error_json`). Auth is
//! applied by whoever composes the router (`require_auth` layered in
//! `start_with_routes` callers), exactly as the other route modules.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use pocket_agents::state::{ApprovalError, Approvals, MemJournal, Outcome, SessionRule};
use pocket_agents::types::{now_ms, AgentSession, ApprovalState, Decision, Risk};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth::pocket_error_json;

const MAX_DIFF_LINES: usize = 400;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Running,
    Stopped,
}

/// One tracked agent session. `pocket_agents::AgentSession` has no status
/// field yet, so the server wraps it; there is no live process here (the
/// bridge owns processes).
pub struct SessionEntry {
    pub session: AgentSession,
    pub status: SessionStatus,
}

/// Shared app-state for the agents API: the AGT-02 state machine plus the
/// tracked sessions. Whoever owns the bridge inserts sessions/approvals.
// ponytail: MemJournal + rules_disabled flag; swap for the pocket-store
// journal and a real rule gate when AGT-07/08 wire the bridge in.
#[derive(Default)]
pub struct AgentsState {
    pub approvals: Approvals,
    pub sessions: HashMap<String, SessionEntry>,
    pub journal: MemJournal,
    /// Kill switch sets this; auto-approval must consult it before applying
    /// any `SessionRule`.
    pub rules_disabled: bool,
}

impl AgentsState {
    pub fn insert_session(&mut self, session: AgentSession) {
        self.sessions.insert(
            session.id.clone(),
            SessionEntry {
                session,
                status: SessionStatus::Running,
            },
        );
    }
}

pub type SharedAgents = Arc<Mutex<AgentsState>>;

pub fn router(state: SharedAgents) -> Router {
    Router::new()
        .route("/pocket/v1/approvals", get(list_approvals))
        .route(
            "/pocket/v1/approvals/{id}",
            get(get_approval).post(decide_approval),
        )
        .route("/pocket/v1/agents", get(list_agents))
        .route("/pocket/v1/agents/{id}/kill", post(kill_agent))
        .route("/pocket/v1/kill", post(kill_switch))
        .with_state(state)
}

/// The agents.md §5 payload for one approval. `command`, `files`, `reason`
/// and `repo` live in the opaque `detail_json` the bridge recorded; `agent`
/// comes from the session, `repo` falls back to the session cwd's basename.
fn approval_payload(
    a: &pocket_agents::types::Approval,
    sessions: &HashMap<String, SessionEntry>,
) -> Value {
    let detail: Value = serde_json::from_str(&a.detail_json).unwrap_or(Value::Null);
    let session = sessions.get(&a.session_id).map(|e| &e.session);

    let mut files = detail.get("files").cloned().unwrap_or(Value::Null);
    truncate_diffs(&mut files);

    let repo = detail.get("repo").cloned().unwrap_or_else(|| {
        session
            .and_then(|s| std::path::Path::new(&s.cwd).file_name())
            .map(|n| json!(n.to_string_lossy()))
            .unwrap_or(Value::Null)
    });

    let mut options = vec!["approve_once"];
    if a.risk != Risk::High {
        options.push("approve_session");
    }
    options.extend(["deny", "deny_and_stop"]);

    json!({
        "id": a.id,
        "session_id": a.session_id,
        "agent": session.map(|s| json!(s.agent)).unwrap_or(Value::Null),
        "repo": repo,
        "kind": a.kind,
        "risk": a.risk,
        "title": a.summary,
        "command": detail.get("command").cloned().unwrap_or(Value::Null),
        "files": files,
        "reason": detail.get("reason").cloned().unwrap_or(Value::Null),
        "options": options,
        "created_at": a.created_at,
        "expires_at": a.expires_at,
    })
}

/// Diffs > 400 lines are truncated with `truncated: true` (agents.md §5);
/// the Mac UI fetches the full diff from the bridge on demand.
fn truncate_diffs(files: &mut Value) {
    let Some(arr) = files.as_array_mut() else {
        return;
    };
    for f in arr {
        let Some(diff) = f.get("diff").and_then(Value::as_str) else {
            continue;
        };
        let lines: Vec<&str> = diff.lines().collect();
        if lines.len() > MAX_DIFF_LINES {
            f["diff"] = Value::String(lines[..MAX_DIFF_LINES].join("\n"));
            f["truncated"] = Value::Bool(true);
        }
    }
}

async fn list_approvals(State(state): State<SharedAgents>) -> Json<Value> {
    let mut state = state.lock().unwrap();
    let now = now_ms();
    let AgentsState {
        approvals, journal, ..
    } = &mut *state;
    approvals.expire_due(journal, now);
    let data: Vec<Value> = state
        .approvals
        .pending()
        .map(|a| approval_payload(a, &state.sessions))
        .collect();
    Json(json!({ "object": "list", "data": data }))
}

async fn get_approval(State(state): State<SharedAgents>, Path(id): Path<String>) -> Response {
    let mut state = state.lock().unwrap();
    let AgentsState {
        approvals, journal, ..
    } = &mut *state;
    approvals.expire_due(journal, now_ms());
    let Some(a) = state.approvals.get(&id) else {
        return error(StatusCode::NOT_FOUND, "approval not found", "not_found");
    };
    let mut body = approval_payload(a, &state.sessions);
    body["state"] = json!(a.state);
    body["decision"] = json!(a.decision);
    Json(body).into_response()
}

#[derive(Deserialize)]
struct RuleBody {
    command_prefix: Option<String>,
    file_glob: Option<String>,
}

#[derive(Deserialize)]
struct DecisionBody {
    decision: Decision,
    idempotency_key: Option<String>,
    rule: Option<RuleBody>,
}

fn error(status: StatusCode, message: &str, code: &str) -> Response {
    (status, Json(pocket_error_json(message, code))).into_response()
}

fn outcome_json(id: &str, out: Outcome) -> Value {
    json!({ "id": id, "state": out.state, "decision": out.decision })
}

async fn decide_approval(
    State(state): State<SharedAgents>,
    Path(id): Path<String>,
    Json(body): Json<DecisionBody>,
) -> Response {
    let key = match body.idempotency_key.as_deref() {
        Some(k) if !k.is_empty() => k.to_string(),
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "idempotency_key is required",
                "missing_idempotency_key",
            )
        }
    };
    let rule = body.rule.and_then(|r| {
        r.command_prefix
            .map(SessionRule::CommandPrefix)
            .or(r.file_glob.map(SessionRule::FileGlob))
    });

    let mut state = state.lock().unwrap();
    // Same key + same decision replays; same key + another decision is a
    // conflicting reuse, never silently mapped onto the first outcome.
    if let Some(a) = state.approvals.get(&id) {
        if a.decision_key.as_deref() == Some(key.as_str()) && a.decision != Some(body.decision) {
            return error(
                StatusCode::CONFLICT,
                "idempotency_key already used for a different decision",
                "idempotency_key_reused",
            );
        }
    }
    let AgentsState {
        approvals, journal, ..
    } = &mut *state;
    match approvals.decide(journal, &id, body.decision, &key, rule, now_ms()) {
        // Expired outcome (nobody decided in time): 409, body still carries
        // the recorded outcome so the card can update (agents.md §7).
        Ok(out) if out.state == ApprovalState::Expired => {
            let mut body = pocket_error_json("approval already expired", "expired");
            body["outcome"] = outcome_json(&id, out);
            (StatusCode::CONFLICT, Json(body)).into_response()
        }
        // First decision and every duplicate (same or different key): 200
        // with the first recorded outcome.
        Ok(out) => Json(outcome_json(&id, out)).into_response(),
        Err(ApprovalError::NotFound(_)) => {
            error(StatusCode::NOT_FOUND, "approval not found", "not_found")
        }
        Err(ApprovalError::SessionApprovalForbidden) => error(
            StatusCode::BAD_REQUEST,
            "approve_session is never allowed for high-risk approvals",
            "session_approval_forbidden",
        ),
        Err(ApprovalError::MissingRule) => error(
            StatusCode::BAD_REQUEST,
            "approve_session requires a rule",
            "missing_rule",
        ),
    }
}

fn session_json(e: &SessionEntry) -> Value {
    json!({
        "id": e.session.id,
        "agent": e.session.agent,
        "cwd": e.session.cwd,
        "policy": e.session.policy,
        "status": e.status,
        "created_at": e.session.created_at,
    })
}

async fn list_agents(State(state): State<SharedAgents>) -> Json<Value> {
    let state = state.lock().unwrap();
    let mut data: Vec<&SessionEntry> = state.sessions.values().collect();
    data.sort_by_key(|e| e.session.created_at);
    Json(json!({
        "object": "list",
        "data": data.iter().map(|e| session_json(e)).collect::<Vec<_>>(),
    }))
}

/// Stops one session: status → `stopped`, pending approvals for it
/// cancelled (agents.md §7 "pending approvals cancelled").
async fn kill_agent(State(state): State<SharedAgents>, Path(id): Path<String>) -> Response {
    let mut state = state.lock().unwrap();
    let Some(entry) = state.sessions.get_mut(&id) else {
        return error(StatusCode::NOT_FOUND, "session not found", "not_found");
    };
    entry.status = SessionStatus::Stopped;
    // TODO(AGT-08): propagate to bridge (the bridge owns the live process).

    let pending: Vec<String> = state
        .approvals
        .pending()
        .filter(|a| a.session_id == id)
        .map(|a| a.id.clone())
        .collect();
    let now = now_ms();
    let AgentsState {
        approvals, journal, ..
    } = &mut *state;
    for aid in &pending {
        let _ = approvals.cancel(journal, aid, now);
    }
    let body = session_json(&state.sessions[&id]);
    Json(body).into_response()
}

/// Kill switch (agents.md §7): stop all sessions, deny all pending
/// approvals, disable "Allow for this session" rules.
async fn kill_switch(State(state): State<SharedAgents>) -> Json<Value> {
    let mut state = state.lock().unwrap();
    let now = now_ms();

    let mut sessions_stopped = 0;
    for entry in state.sessions.values_mut() {
        if entry.status == SessionStatus::Running {
            entry.status = SessionStatus::Stopped;
            sessions_stopped += 1;
        }
    }
    // TODO(AGT-08): propagate to bridge (stop the live agent processes).

    let pending: Vec<String> = state.approvals.pending().map(|a| a.id.clone()).collect();
    let AgentsState {
        approvals,
        journal,
        rules_disabled,
        ..
    } = &mut *state;
    for aid in &pending {
        let _ = approvals.decide(journal, aid, Decision::Deny, "kill-switch", None, now);
    }
    *rules_disabled = true;

    Json(json!({
        "sessions_stopped": sessions_stopped,
        "approvals_denied": pending.len(),
        "rules_disabled": true,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use pocket_agents::types::{AgentKind, Approval, ApprovalKind, Policy, DEFAULT_TTL_MS};
    use tower::ServiceExt;

    fn setup() -> (Router, SharedAgents, String, String) {
        let mut s = AgentsState::default();
        let session = AgentSession::new(
            AgentKind::ClaudeCode,
            "/Users/r/code/pocketai-core",
            Policy::AskEveryTime,
        );
        let session_id = session.id.clone();
        s.insert_session(session);

        let detail = json!({
            "command": { "argv": ["git", "push", "origin", "main"], "cwd": "/Users/r/code/pocketai-core" },
            "files": [ { "path": "src/lib.rs", "change": "edit", "diff": "@@ -1 +1 @@", "truncated": false } ],
            "reason": "Push the fix so CI runs",
        });
        let approval = Approval::new(
            &session_id,
            ApprovalKind::Command,
            Risk::Medium,
            "Run a command",
            detail.to_string(),
            now_ms(),
        );
        let approval_id = approval.id.clone();
        s.approvals.insert(approval);

        let state: SharedAgents = Arc::new(Mutex::new(s));
        (router(state.clone()), state, session_id, approval_id)
    }

    async fn request(app: &Router, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(path);
        let body = match body {
            Some(v) => {
                builder = builder.header("content-type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let response = app
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, value)
    }

    #[tokio::test]
    async fn pending_list_has_the_section5_payload_shape() {
        let (app, state, session_id, approval_id) = setup();
        // A second approval with an over-long diff to check truncation.
        let long_diff = (0..500).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        {
            let mut s = state.lock().unwrap();
            s.approvals.insert(Approval::new(
                &session_id,
                ApprovalKind::FileEdit,
                Risk::High,
                "Edit a file",
                json!({ "files": [ { "path": "a.rs", "change": "edit", "diff": long_diff, "truncated": false } ] })
                    .to_string(),
                now_ms(),
            ));
        }

        let (status, body) = request(&app, "GET", "/pocket/v1/approvals", None).await;
        assert_eq!(status, StatusCode::OK);
        let data = body["data"].as_array().unwrap();
        assert_eq!(data.len(), 2);

        let cmd = data.iter().find(|a| a["id"] == approval_id.as_str()).unwrap();
        assert_eq!(cmd["session_id"], session_id);
        assert_eq!(cmd["agent"], "claude_code");
        assert_eq!(cmd["repo"], "pocketai-core");
        assert_eq!(cmd["kind"], "command");
        assert_eq!(cmd["risk"], "medium");
        assert_eq!(cmd["title"], "Run a command");
        assert_eq!(cmd["command"]["argv"][0], "git");
        assert_eq!(cmd["files"][0]["path"], "src/lib.rs");
        assert_eq!(cmd["reason"], "Push the fix so CI runs");
        assert_eq!(
            cmd["options"],
            json!(["approve_once", "approve_session", "deny", "deny_and_stop"])
        );
        assert!(cmd["created_at"].is_i64() && cmd["expires_at"].is_i64());

        // High risk: no approve_session option; long diff truncated to 400 lines.
        let edit = data.iter().find(|a| a["kind"] == "file_edit").unwrap();
        assert_eq!(edit["options"], json!(["approve_once", "deny", "deny_and_stop"]));
        assert_eq!(edit["files"][0]["truncated"], true);
        assert_eq!(
            edit["files"][0]["diff"].as_str().unwrap().lines().count(),
            MAX_DIFF_LINES
        );
    }

    #[tokio::test]
    async fn decision_happy_path_then_duplicate_returns_first_result() {
        let (app, state, _, approval_id) = setup();
        let path = format!("/pocket/v1/approvals/{approval_id}");

        let (status, first) = request(
            &app,
            "POST",
            &path,
            Some(json!({ "decision": "approve_once", "idempotency_key": "key-a" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(first["state"], "approved");
        assert_eq!(first["decision"], "approve_once");

        // Duplicate with a different key and a contrary decision: 200, same body.
        let (status, second) = request(
            &app,
            "POST",
            &path,
            Some(json!({ "decision": "deny", "idempotency_key": "key-b" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(second, first);

        // Settled approvals no longer list as pending.
        let (_, list) = request(&app, "GET", "/pocket/v1/approvals", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 0);
        drop(state);
    }

    #[tokio::test]
    async fn same_key_same_decision_replays_conflicting_reuse_is_409() {
        let (app, state, _, approval_id) = setup();
        let path = format!("/pocket/v1/approvals/{approval_id}");
        let deny = json!({ "decision": "deny", "idempotency_key": "k1" });

        let (status, first) = request(&app, "POST", &path, Some(deny.clone())).await;
        assert_eq!(status, StatusCode::OK);
        let (status, replay) = request(&app, "POST", &path, Some(deny)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(replay, first);

        let (status, body) = request(
            &app,
            "POST",
            &path,
            Some(json!({ "decision": "approve_once", "idempotency_key": "k1" })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["error"]["code"], "idempotency_key_reused");

        // Exactly one transition was journaled: no double-apply.
        let s = state.lock().unwrap();
        assert_eq!(s.journal.0.lock().unwrap().len(), 1);
        assert_eq!(s.approvals.get(&approval_id).unwrap().state, ApprovalState::Denied);
    }

    #[tokio::test]
    async fn get_single_approval_includes_state_and_404s_unknown() {
        let (app, _, _, approval_id) = setup();
        let path = format!("/pocket/v1/approvals/{approval_id}");

        let (status, body) = request(&app, "GET", &path, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["id"], approval_id);
        assert_eq!(body["state"], "pending");
        assert!(body["decision"].is_null());

        request(
            &app,
            "POST",
            &path,
            Some(json!({ "decision": "approve_once", "idempotency_key": "k" })),
        )
        .await;
        let (_, body) = request(&app, "GET", &path, None).await;
        assert_eq!(body["state"], "approved");
        assert_eq!(body["decision"], "approve_once");

        let (status, body) = request(&app, "GET", "/pocket/v1/approvals/nope", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "not_found");
    }

    #[tokio::test]
    async fn routes_sit_behind_require_auth_like_the_other_modules() {
        use crate::auth::{require_auth, AuthStore};
        struct NoDevices;
        impl AuthStore for NoDevices {
            fn device_id_for_token_hash(&self, _: &str) -> Option<String> {
                None
            }
            fn local_api_key_hash(&self) -> Option<String> {
                None
            }
            fn mac_is_pro(&self) -> bool {
                true
            }
        }
        let (app, _, _, _) = setup();
        let store: Arc<dyn AuthStore> = Arc::new(NoDevices);
        let app = app.layer(axum::middleware::from_fn_with_state(store, require_auth));
        let (status, _) = request(&app, "GET", "/pocket/v1/approvals", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn missing_idempotency_key_is_400() {
        let (app, _, _, approval_id) = setup();
        let (status, body) = request(
            &app,
            "POST",
            &format!("/pocket/v1/approvals/{approval_id}"),
            Some(json!({ "decision": "deny" })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "missing_idempotency_key");
    }

    #[tokio::test]
    async fn approve_session_on_high_risk_is_rejected() {
        let (app, state, session_id, _) = setup();
        let high = Approval::new(
            &session_id,
            ApprovalKind::Command,
            Risk::High,
            "Run a command",
            "{}",
            now_ms(),
        );
        let high_id = high.id.clone();
        state.lock().unwrap().approvals.insert(high);

        let (status, body) = request(
            &app,
            "POST",
            &format!("/pocket/v1/approvals/{high_id}"),
            Some(json!({
                "decision": "approve_session",
                "idempotency_key": "k",
                "rule": { "command_prefix": "git status" },
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "session_approval_forbidden");
        // Still pending: no transition happened.
        assert_eq!(
            state.lock().unwrap().approvals.get(&high_id).unwrap().state,
            ApprovalState::Pending
        );
    }

    #[tokio::test]
    async fn expired_approval_decision_is_409_with_expired_outcome() {
        let (app, state, session_id, _) = setup();
        let mut overdue = Approval::new(
            &session_id,
            ApprovalKind::Command,
            Risk::Low,
            "Old",
            "{}",
            now_ms() - 2 * DEFAULT_TTL_MS,
        );
        overdue.expires_at = now_ms() - DEFAULT_TTL_MS;
        let overdue_id = overdue.id.clone();
        state.lock().unwrap().approvals.insert(overdue);

        let (status, body) = request(
            &app,
            "POST",
            &format!("/pocket/v1/approvals/{overdue_id}"),
            Some(json!({ "decision": "approve_once", "idempotency_key": "k" })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["error"]["code"], "expired");
        assert_eq!(body["outcome"]["state"], "expired");
        assert!(body["outcome"]["decision"].is_null());
    }

    #[tokio::test]
    async fn agents_list_and_single_kill() {
        let (app, state, session_id, approval_id) = setup();

        let (status, body) = request(&app, "GET", "/pocket/v1/agents", None).await;
        assert_eq!(status, StatusCode::OK);
        let agent = &body["data"][0];
        assert_eq!(agent["id"], session_id);
        assert_eq!(agent["status"], "running");

        let (status, body) = request(
            &app,
            "POST",
            &format!("/pocket/v1/agents/{session_id}/kill"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "stopped");
        // Its pending approval was cancelled.
        assert_eq!(
            state.lock().unwrap().approvals.get(&approval_id).unwrap().state,
            ApprovalState::Cancelled
        );

        let (status, body) = request(&app, "POST", "/pocket/v1/agents/nope/kill", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "not_found");
    }

    #[tokio::test]
    async fn kill_switch_denies_all_pending_and_stops_everything() {
        let (app, state, session_id, approval_id) = setup();
        let second = Approval::new(&session_id, ApprovalKind::Other, Risk::Low, "x", "{}", now_ms());
        let second_id = second.id.clone();
        state.lock().unwrap().approvals.insert(second);

        let (status, body) = request(&app, "POST", "/pocket/v1/kill", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["sessions_stopped"], 1);
        assert_eq!(body["approvals_denied"], 2);
        assert_eq!(body["rules_disabled"], true);

        {
            let s = state.lock().unwrap();
            for id in [&approval_id, &second_id] {
                let a = s.approvals.get(id).unwrap();
                assert_eq!(a.state, ApprovalState::Denied);
                assert_eq!(a.decision, Some(Decision::Deny));
            }
            assert!(s.rules_disabled);
            assert_eq!(s.sessions[&session_id].status, SessionStatus::Stopped);
        }

        let (_, list) = request(&app, "GET", "/pocket/v1/approvals", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 0);
    }
}
