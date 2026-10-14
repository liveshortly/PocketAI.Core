use serde::{Deserialize, Serialize};

use crate::Id;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: Id,
    pub name: String,
    pub instructions: Option<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub id: Id,
    pub project_id: Id,
    pub name: String,
    pub mime: String,
    pub bytes: u64,
    pub path: String,
    pub text_chars: u32,
    pub embed_model: Option<String>,
    pub state: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChunkHit {
    pub document_id: Id,
    pub ord: u32,
    pub text: String,
    pub page: Option<u32>,
    pub score: f32,
}
