//! UI-68: edit a sent message in place (Mac chat).

use std::sync::Arc;

use crate::core::PocketCore;
use crate::error::PocketError;
use crate::traits::ChatSink;
use crate::types::*;

#[uniffi::export]
impl PocketCore {
    /// Replaces the user message `message_id` with `input` and answers it again.
    /// The message and everything after it (replies, regenerated siblings) are
    /// deleted first, as synced deletes, then this runs like `send_with_reasoning`.
    /// Only a user message can be edited, and the caller should have checked a
    /// model is available: a failure after the delete leaves the history cut.
    pub fn edit_message(
        &self,
        conversation_id: Id,
        message_id: Id,
        input: UserInput,
        reasoning: ReasoningMode,
        sink: Arc<dyn ChatSink>,
    ) -> Result<Message, PocketError> {
        let repo = pocket_store::MessagesRepo::new(&self.store);
        let target = repo
            .get(&message_id)
            .map_err(|e| PocketError::Storage { message: e.to_string() })?
            .filter(|m| m.conversation_id == conversation_id && m.role == "user")
            .ok_or_else(|| PocketError::InvalidInput {
                message: "only a message you sent can be edited".into(),
            })?;
        repo.delete_from(&target.id)
            .map_err(|e| PocketError::Storage { message: e.to_string() })?;
        self.send_with_reasoning(conversation_id, input, reasoning, sink)
    }
}
