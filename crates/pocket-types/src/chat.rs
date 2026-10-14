use serde::{Deserialize, Serialize};

use crate::Id;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ContentPart {
    Text { text: String },
    ImageRef { path: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub parts: Vec<ContentPart>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters_json_schema: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCallDelta {
    pub id: String,
    pub name: String,
    pub arguments_json_fragment: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ReasoningMode {
    Off,
    On,
    Auto,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SamplingParams {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: i32,
    pub min_p: f32,
    pub max_tokens: u32,
    pub stop: Vec<String>,
    pub seed: Option<i64>,
    pub repetition_penalty: f32,
}

impl Default for SamplingParams {
    fn default() -> Self {
        Self {
            temperature: 0.7,
            top_p: 0.95,
            top_k: 40,
            min_p: 0.0,
            max_tokens: 1024,
            stop: Vec::new(),
            seed: None,
            repetition_penalty: 1.1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenerateRequest {
    pub request_id: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSpec>,
    pub params: SamplingParams,
    pub reasoning: ReasoningMode,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GenerateDelta {
    pub text: String,
    pub reasoning: String,
    pub tool_calls: Vec<ToolCallDelta>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Cancelled,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenerateStats {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub ttft_ms: u32,
    pub decode_tps: f32,
    pub finish: FinishReason,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserInput {
    pub parts: Vec<ContentPart>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MessageStatus {
    Streaming,
    Done,
    Error,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: Id,
    pub conversation_id: Id,
    pub role: String,
    pub parts: Vec<ContentPart>,
    pub reasoning: Option<String>,
    pub stats: Option<GenerateStats>,
    pub model_id: Option<String>,
    pub created_at: i64,
    pub status: MessageStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: Id,
    pub title: Option<String>,
    pub project_id: Option<Id>,
    pub model_id: Option<String>,
    pub system_prompt: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConversationSummary {
    pub id: Id,
    pub title: Option<String>,
    pub updated_at: i64,
    pub archived: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ConversationQuery {
    pub search: Option<String>,
    pub archived: Option<bool>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewConversation {
    pub title: Option<String>,
    pub project_id: Option<Id>,
    pub model_id: Option<String>,
}
