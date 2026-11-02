//! Agent sessions and the approval state machine (AGT-01, AGT-02).
//! Risk classification lives in [`risk`] (AGT-03).

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

pub mod risk;
pub mod state;
pub mod types;

pub use state::{Approvals, ApprovalError, Journal, MemJournal, Outcome, SessionRule, TransitionEvent};
pub use types::*;
