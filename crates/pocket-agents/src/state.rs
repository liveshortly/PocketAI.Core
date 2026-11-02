//! Approval state machine (AGT-02).
//!
//! `pending → approved | denied | expired | cancelled`, all terminal.
//! Pure: callers inject `now_ms`, every transition goes through a [`Journal`]
//! (pocket-store-backed in pocket-server), and [`Approvals`] is serde-serializable
//! so pending approvals reload across restarts with their remaining TTL.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::{Approval, ApprovalState, Decision, Risk};

/// Sink for every state transition (pocket-server's store journal writes
/// `agent_events` + `audit`).
pub trait Journal {
    fn record(&self, event: &TransitionEvent);
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionEvent {
    pub approval_id: String,
    pub session_id: String,
    pub from: ApprovalState,
    pub to: ApprovalState,
    pub decision: Option<Decision>,
    pub decision_key: Option<String>,
    pub at_ms: i64,
}

/// In-memory journal for tests.
#[derive(Debug, Default)]
pub struct MemJournal(pub std::sync::Mutex<Vec<TransitionEvent>>);

impl Journal for MemJournal {
    fn record(&self, event: &TransitionEvent) {
        self.0.lock().unwrap().push(event.clone());
    }
}

/// The recorded end state of an approval; what a (possibly duplicate)
/// decision call gets back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    pub state: ApprovalState,
    /// `None` when settled by expiry or cancellation.
    pub decision: Option<Decision>,
}

/// Session-scoped auto-approve rule stored by [`Decision::ApproveSession`].
/// Exact command prefix or one file glob inside the repo — never wildcards
/// beyond that, never for High risk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionRule {
    CommandPrefix(String),
    FileGlob(String),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ApprovalError {
    #[error("approval not found: {0}")]
    NotFound(String),
    #[error("ApproveSession is never allowed for high-risk approvals")]
    SessionApprovalForbidden,
    #[error("ApproveSession requires a session rule")]
    MissingRule,
}

/// All approvals plus session rules. Serialize/deserialize the whole thing to
/// persist across restarts.
// ponytail: plain in-memory maps persisted as one JSON snapshot by
// pocket-server; per-row tables when volumes or multi-process access demand it.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Approvals {
    approvals: HashMap<String, Approval>,
    session_rules: HashMap<String, Vec<SessionRule>>,
}

impl Approvals {
    /// Registers a new pending approval. No journal entry: creation is not a
    /// transition (the server emits `approval.requested` itself).
    pub fn insert(&mut self, approval: Approval) {
        self.approvals.insert(approval.id.clone(), approval);
    }

    pub fn get(&self, id: &str) -> Option<&Approval> {
        self.approvals.get(id)
    }

    pub fn pending(&self) -> impl Iterator<Item = &Approval> {
        self.approvals
            .values()
            .filter(|a| a.state == ApprovalState::Pending)
    }

    /// Rules stored by `ApproveSession` decisions for one session.
    pub fn rules(&self, session_id: &str) -> &[SessionRule] {
        self.session_rules
            .get(session_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Drops every session rule (kill switch, agents.md §7). Later
    /// `ApproveSession` decisions may store new ones.
    pub fn clear_rules(&mut self) {
        self.session_rules.clear();
    }

    /// Applies a decision. Idempotent: once settled (by any decision, expiry,
    /// or cancellation), every later call — whatever its key — returns the
    /// recorded [`Outcome`] without transitioning again. A pending approval
    /// past its `expires_at` expires first, so the decision gets the expired
    /// outcome. `rule` is required for `ApproveSession`, which is rejected
    /// outright for High risk (no transition).
    pub fn decide(
        &mut self,
        journal: &dyn Journal,
        id: &str,
        decision: Decision,
        idempotency_key: &str,
        rule: Option<SessionRule>,
        now_ms: i64,
    ) -> Result<Outcome, ApprovalError> {
        let approval = self
            .approvals
            .get_mut(id)
            .ok_or_else(|| ApprovalError::NotFound(id.to_string()))?;

        if approval.state == ApprovalState::Pending && now_ms >= approval.expires_at {
            transition(journal, approval, ApprovalState::Expired, None, None, now_ms);
        }
        if approval.state != ApprovalState::Pending {
            return Ok(outcome_of(approval));
        }

        let to = match decision {
            Decision::ApproveSession => {
                if approval.risk == Risk::High {
                    return Err(ApprovalError::SessionApprovalForbidden);
                }
                if rule.is_none() {
                    return Err(ApprovalError::MissingRule);
                }
                ApprovalState::Approved
            }
            Decision::ApproveOnce => ApprovalState::Approved,
            Decision::Deny | Decision::DenyAndStop => ApprovalState::Denied,
        };
        if decision == Decision::ApproveSession {
            self.session_rules
                .entry(approval.session_id.clone())
                .or_default()
                .push(rule.expect("checked above"));
        }
        transition(
            journal,
            approval,
            to,
            Some(decision),
            Some(idempotency_key.to_string()),
            now_ms,
        );
        Ok(outcome_of(approval))
    }

    /// Expires every pending approval whose TTL has elapsed. Returns the ids
    /// expired. Call periodically (or on reload).
    pub fn expire_due(&mut self, journal: &dyn Journal, now_ms: i64) -> Vec<String> {
        let mut expired = Vec::new();
        for approval in self.approvals.values_mut() {
            if approval.state == ApprovalState::Pending && now_ms >= approval.expires_at {
                transition(journal, approval, ApprovalState::Expired, None, None, now_ms);
                expired.push(approval.id.clone());
            }
        }
        expired
    }

    /// Cancels a pending approval (agent crashed / session stopped). Settled
    /// approvals are left as recorded.
    pub fn cancel(
        &mut self,
        journal: &dyn Journal,
        id: &str,
        now_ms: i64,
    ) -> Result<Outcome, ApprovalError> {
        let approval = self
            .approvals
            .get_mut(id)
            .ok_or_else(|| ApprovalError::NotFound(id.to_string()))?;
        if approval.state == ApprovalState::Pending {
            transition(
                journal,
                approval,
                ApprovalState::Cancelled,
                None,
                None,
                now_ms,
            );
        }
        Ok(outcome_of(approval))
    }
}

fn outcome_of(a: &Approval) -> Outcome {
    Outcome {
        state: a.state,
        decision: a.decision,
    }
}

fn transition(
    journal: &dyn Journal,
    approval: &mut Approval,
    to: ApprovalState,
    decision: Option<Decision>,
    decision_key: Option<String>,
    at_ms: i64,
) {
    let event = TransitionEvent {
        approval_id: approval.id.clone(),
        session_id: approval.session_id.clone(),
        from: approval.state,
        to,
        decision,
        decision_key: decision_key.clone(),
        at_ms,
    };
    approval.state = to;
    approval.decision = decision;
    approval.decision_key = decision_key;
    journal.record(&event);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ApprovalKind, DEFAULT_TTL_MS};

    const T0: i64 = 1_000_000;

    fn pending(risk: Risk) -> Approval {
        Approval::new("sess-1", ApprovalKind::Command, risk, "Run a command", "{}", T0)
    }

    fn setup(risk: Risk) -> (Approvals, MemJournal, String) {
        let mut approvals = Approvals::default();
        let a = pending(risk);
        let id = a.id.clone();
        approvals.insert(a);
        (approvals, MemJournal::default(), id)
    }

    #[test]
    fn no_double_decision_second_key_gets_first_outcome() {
        let (mut approvals, journal, id) = setup(Risk::Medium);

        let first = approvals
            .decide(&journal, &id, Decision::ApproveOnce, "key-a", None, T0 + 1)
            .unwrap();
        assert_eq!(first.state, ApprovalState::Approved);
        assert_eq!(first.decision, Some(Decision::ApproveOnce));

        // Different decision, different key: no second transition, first
        // recorded outcome returned.
        let second = approvals
            .decide(&journal, &id, Decision::Deny, "key-b", None, T0 + 2)
            .unwrap();
        assert_eq!(second, first);
        // Same key replay too.
        let replay = approvals
            .decide(&journal, &id, Decision::ApproveOnce, "key-a", None, T0 + 3)
            .unwrap();
        assert_eq!(replay, first);

        let events = journal.0.lock().unwrap();
        assert_eq!(events.len(), 1, "exactly one transition journaled");
        assert_eq!(events[0].from, ApprovalState::Pending);
        assert_eq!(events[0].to, ApprovalState::Approved);
        assert_eq!(events[0].decision_key.as_deref(), Some("key-a"));
        assert_eq!(
            approvals.get(&id).unwrap().decision_key.as_deref(),
            Some("key-a")
        );
    }

    #[test]
    fn expiry_wins_over_late_decision_and_sticks() {
        let (mut approvals, journal, id) = setup(Risk::Medium);

        // Decision lands after the TTL: approval expires first, decision gets
        // the expired outcome.
        let late = T0 + DEFAULT_TTL_MS;
        let out = approvals
            .decide(&journal, &id, Decision::ApproveOnce, "key-a", None, late)
            .unwrap();
        assert_eq!(
            out,
            Outcome {
                state: ApprovalState::Expired,
                decision: None
            }
        );
        // Every later decision gets the recorded expired outcome.
        let again = approvals
            .decide(&journal, &id, Decision::Deny, "key-b", None, late + 1)
            .unwrap();
        assert_eq!(again, out);

        let events = journal.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].to, ApprovalState::Expired);
        assert_eq!(events[0].decision, None);
    }

    #[test]
    fn expire_due_only_expires_elapsed_pending() {
        let mut approvals = Approvals::default();
        let journal = MemJournal::default();
        let due = pending(Risk::Low);
        let mut fresh = pending(Risk::Low);
        fresh.expires_at = T0 + 2 * DEFAULT_TTL_MS;
        let (due_id, fresh_id) = (due.id.clone(), fresh.id.clone());
        approvals.insert(due);
        approvals.insert(fresh);

        let expired = approvals.expire_due(&journal, T0 + DEFAULT_TTL_MS);
        assert_eq!(expired, vec![due_id.clone()]);
        assert_eq!(approvals.get(&due_id).unwrap().state, ApprovalState::Expired);
        assert_eq!(approvals.get(&fresh_id).unwrap().state, ApprovalState::Pending);
        // Second sweep is a no-op (no double transition).
        assert!(approvals.expire_due(&journal, T0 + DEFAULT_TTL_MS).is_empty());
        assert_eq!(journal.0.lock().unwrap().len(), 1);
    }

    #[test]
    fn approve_session_rejected_for_high_risk() {
        let (mut approvals, journal, id) = setup(Risk::High);

        let err = approvals
            .decide(
                &journal,
                &id,
                Decision::ApproveSession,
                "key-a",
                Some(SessionRule::CommandPrefix("git status".into())),
                T0 + 1,
            )
            .unwrap_err();
        assert_eq!(err, ApprovalError::SessionApprovalForbidden);
        // No transition: still pending, no journal entry, no rule stored.
        assert_eq!(approvals.get(&id).unwrap().state, ApprovalState::Pending);
        assert!(journal.0.lock().unwrap().is_empty());
        assert!(approvals.rules("sess-1").is_empty());

        // ApproveOnce on the same high-risk approval still works.
        let out = approvals
            .decide(&journal, &id, Decision::ApproveOnce, "key-b", None, T0 + 2)
            .unwrap();
        assert_eq!(out.state, ApprovalState::Approved);
    }

    #[test]
    fn approve_session_stores_rule_and_requires_one() {
        let (mut approvals, journal, id) = setup(Risk::Medium);

        assert_eq!(
            approvals
                .decide(&journal, &id, Decision::ApproveSession, "k", None, T0 + 1)
                .unwrap_err(),
            ApprovalError::MissingRule
        );

        let rule = SessionRule::FileGlob("src/**/*.rs".into());
        let out = approvals
            .decide(
                &journal,
                &id,
                Decision::ApproveSession,
                "k",
                Some(rule.clone()),
                T0 + 2,
            )
            .unwrap();
        assert_eq!(out.state, ApprovalState::Approved);
        assert_eq!(out.decision, Some(Decision::ApproveSession));
        assert_eq!(approvals.rules("sess-1"), &[rule]);
        assert!(approvals.rules("other-session").is_empty());
    }

    #[test]
    fn cancel_pending_is_terminal_and_idempotent() {
        let (mut approvals, journal, id) = setup(Risk::Low);

        let out = approvals.cancel(&journal, &id, T0 + 1).unwrap();
        assert_eq!(out.state, ApprovalState::Cancelled);
        // Later decision returns the cancelled outcome.
        let late = approvals
            .decide(&journal, &id, Decision::ApproveOnce, "k", None, T0 + 2)
            .unwrap();
        assert_eq!(late, out);
        assert_eq!(journal.0.lock().unwrap().len(), 1);
    }

    #[test]
    fn clear_rules_drops_every_session_rule() {
        let (mut approvals, journal, id) = setup(Risk::Medium);
        approvals
            .decide(
                &journal,
                &id,
                Decision::ApproveSession,
                "k",
                Some(SessionRule::CommandPrefix("cargo test".into())),
                T0 + 1,
            )
            .unwrap();
        assert_eq!(approvals.rules("sess-1").len(), 1);
        approvals.clear_rules();
        assert!(approvals.rules("sess-1").is_empty());
        // Approval state is untouched.
        assert_eq!(approvals.get(&id).unwrap().state, ApprovalState::Approved);
    }

    #[test]
    fn unknown_id_is_not_found() {
        let mut approvals = Approvals::default();
        let journal = MemJournal::default();
        assert_eq!(
            approvals
                .decide(&journal, "nope", Decision::Deny, "k", None, T0)
                .unwrap_err(),
            ApprovalError::NotFound("nope".into())
        );
    }

    #[test]
    fn reload_keeps_pending_with_remaining_ttl() {
        let (mut approvals, journal, id) = setup(Risk::Medium);
        // One settled approval too, to check rules/state survive.
        let other = pending(Risk::Medium);
        let other_id = other.id.clone();
        approvals.insert(other);
        approvals
            .decide(
                &journal,
                &other_id,
                Decision::ApproveSession,
                "k",
                Some(SessionRule::CommandPrefix("cargo test".into())),
                T0 + 1,
            )
            .unwrap();

        // "Restart": serialize, drop, deserialize.
        let blob = serde_json::to_string(&approvals).unwrap();
        drop(approvals);
        let mut reloaded: Approvals = serde_json::from_str(&blob).unwrap();

        let a = reloaded.get(&id).unwrap();
        assert_eq!(a.state, ApprovalState::Pending);
        assert_eq!(a.expires_at, T0 + DEFAULT_TTL_MS, "TTL deadline unchanged");
        assert_eq!(reloaded.get(&other_id).unwrap().state, ApprovalState::Approved);
        assert_eq!(
            reloaded.rules("sess-1"),
            &[SessionRule::CommandPrefix("cargo test".into())]
        );

        // Still decidable within the remaining TTL…
        let out = reloaded
            .decide(
                &journal,
                &id,
                Decision::Deny,
                "k2",
                None,
                T0 + DEFAULT_TTL_MS - 1,
            )
            .unwrap();
        assert_eq!(out.state, ApprovalState::Denied);
    }

    #[test]
    fn reload_then_sweep_expires_overdue_pending() {
        let (mut approvals, journal, id) = setup(Risk::Medium);
        let blob = serde_json::to_string(&approvals).unwrap();
        let mut reloaded: Approvals = serde_json::from_str(&blob).unwrap();
        // Restart happened after the deadline: sweep expires it.
        let expired = reloaded.expire_due(&journal, T0 + DEFAULT_TTL_MS + 5);
        assert_eq!(expired, vec![id.clone()]);
        assert_eq!(reloaded.get(&id).unwrap().state, ApprovalState::Expired);
    }
}
