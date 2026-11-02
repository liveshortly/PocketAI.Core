//! Agent and approval types (AGT-01). Serde names match the shared approval
//! payload in `contracts/events.schema.json` ("claude_code", "command", "high").
//!
//! uniffi derives are behind the `uniffi` feature (D19: scaffolding is
//! per-crate; pocket-ffi re-exports these later).

use serde::{Deserialize, Serialize};

/// Default approval TTL: 600 s, after which a pending approval expires
/// (= deny to the agent).
pub const DEFAULT_TTL_MS: i64 = 600_000;

/// Unix epoch milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum AgentKind {
    ClaudeCode,
    Codex,
    OpenCode,
    Goose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ApprovalKind {
    Command,
    FileEdit,
    FileCreate,
    FileDelete,
    Network,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Risk {
    Low,
    Medium,
    High,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Policy {
    AskEveryTime,
    AllowReads,
    AllowInRepoEdits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Decision {
    ApproveOnce,
    ApproveSession,
    Deny,
    DenyAndStop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ApprovalState {
    Pending,
    Approved,
    Denied,
    Expired,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AgentSession {
    pub id: String,
    pub agent: AgentKind,
    /// Repo root the agent runs in.
    pub cwd: String,
    pub policy: Policy,
    pub created_at: i64,
}

impl AgentSession {
    pub fn new(agent: AgentKind, cwd: impl Into<String>, policy: Policy) -> Self {
        Self {
            id: pocket_types::new_id(),
            agent,
            cwd: cwd.into(),
            policy,
            created_at: now_ms(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Approval {
    pub id: String,
    pub session_id: String,
    pub kind: ApprovalKind,
    pub risk: Risk,
    /// Short human title ("Run a command").
    pub summary: String,
    /// Raw payload (command argv, file diffs) as JSON; opaque to this crate.
    pub detail_json: String,
    pub state: ApprovalState,
    /// Recorded decision once settled by one (approved/denied only).
    pub decision: Option<Decision>,
    /// Idempotency key of the decision that settled it; later keys get the
    /// recorded outcome, never a second transition.
    pub decision_key: Option<String>,
    pub created_at: i64,
    pub expires_at: i64,
}

impl Approval {
    /// New pending approval with the default 600 s TTL from `now_ms`
    /// (injected for testability). Set `expires_at` directly for other TTLs.
    pub fn new(
        session_id: impl Into<String>,
        kind: ApprovalKind,
        risk: Risk,
        summary: impl Into<String>,
        detail_json: impl Into<String>,
        now_ms: i64,
    ) -> Self {
        Self {
            id: pocket_types::new_id(),
            session_id: session_id.into(),
            kind,
            risk,
            summary: summary.into(),
            detail_json: detail_json.into(),
            state: ApprovalState::Pending,
            decision: None,
            decision_key: None,
            created_at: now_ms,
            expires_at: now_ms + DEFAULT_TTL_MS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(v: &T) {
        let json = serde_json::to_string(v).unwrap();
        assert_eq!(&serde_json::from_str::<T>(&json).unwrap(), v);
    }

    #[test]
    fn enums_roundtrip_and_use_contract_names() {
        for k in [
            AgentKind::ClaudeCode,
            AgentKind::Codex,
            AgentKind::OpenCode,
            AgentKind::Goose,
        ] {
            roundtrip(&k);
        }
        for k in [
            ApprovalKind::Command,
            ApprovalKind::FileEdit,
            ApprovalKind::FileCreate,
            ApprovalKind::FileDelete,
            ApprovalKind::Network,
            ApprovalKind::Other,
        ] {
            roundtrip(&k);
        }
        for r in [Risk::Low, Risk::Medium, Risk::High] {
            roundtrip(&r);
        }
        for p in [
            Policy::AskEveryTime,
            Policy::AllowReads,
            Policy::AllowInRepoEdits,
        ] {
            roundtrip(&p);
        }
        for d in [
            Decision::ApproveOnce,
            Decision::ApproveSession,
            Decision::Deny,
            Decision::DenyAndStop,
        ] {
            roundtrip(&d);
        }
        for s in [
            ApprovalState::Pending,
            ApprovalState::Approved,
            ApprovalState::Denied,
            ApprovalState::Expired,
            ApprovalState::Cancelled,
        ] {
            roundtrip(&s);
        }
        // Contract spellings from 08-agents-spec §5.
        assert_eq!(
            serde_json::to_string(&AgentKind::ClaudeCode).unwrap(),
            "\"claude_code\""
        );
        assert_eq!(
            serde_json::to_string(&ApprovalKind::Command).unwrap(),
            "\"command\""
        );
        assert_eq!(serde_json::to_string(&Risk::High).unwrap(), "\"high\"");
        assert_eq!(
            serde_json::to_string(&Decision::ApproveOnce).unwrap(),
            "\"approve_once\""
        );
    }

    #[test]
    fn structs_roundtrip() {
        let session = AgentSession::new(AgentKind::Codex, "/tmp/repo", Policy::AllowReads);
        roundtrip(&session);

        let approval = Approval::new(
            &session.id,
            ApprovalKind::Command,
            Risk::Medium,
            "Run a command",
            r#"{"argv":["git","status"]}"#,
            1_760_430_000_123,
        );
        roundtrip(&approval);
        assert_eq!(approval.state, ApprovalState::Pending);
        assert_eq!(approval.expires_at - approval.created_at, DEFAULT_TTL_MS);
    }

    #[test]
    fn risk_orders_low_to_high() {
        assert!(Risk::Low < Risk::Medium && Risk::Medium < Risk::High);
    }
}
