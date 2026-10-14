use serde::{Deserialize, Serialize};
use ulid::Ulid;

pub type Id = String;

pub fn new_id() -> Id {
    Ulid::new().to_string()
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelId {
    pub value: String,
}

impl ModelId {
    /// "mlx-community/Qwen3-4B-Instruct-4bit@<rev>" or "mac:<device>/<repo>@<rev>"
    pub fn parse(s: &str) -> Option<Self> {
        if s.is_empty() || !s.contains('@') {
            return None;
        }
        Some(Self {
            value: s.to_string(),
        })
    }

    pub fn is_remote(&self) -> bool {
        self.value.starts_with("mac:")
    }

    pub fn repo(&self) -> &str {
        let body = self
            .value
            .strip_prefix("mac:")
            .map(|rest| rest.split_once('/').map(|(_, r)| r).unwrap_or(rest))
            .unwrap_or(&self.value);
        body.split_once('@').map(|(repo, _)| repo).unwrap_or(body)
    }

    pub fn revision(&self) -> &str {
        self.value
            .rsplit_once('@')
            .map(|(_, rev)| rev)
            .unwrap_or("")
    }
}

impl std::fmt::Display for ModelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_local_model_id() {
        let id = ModelId::parse("mlx-community/Qwen3-4B-Instruct-4bit@abc123").unwrap();
        assert!(!id.is_remote());
        assert_eq!(id.repo(), "mlx-community/Qwen3-4B-Instruct-4bit");
        assert_eq!(id.revision(), "abc123");
    }

    #[test]
    fn parses_remote_model_id() {
        let id = ModelId::parse("mac:01J.../mlx-community/Qwen3-8B-4bit@def456").unwrap();
        assert!(id.is_remote());
        assert_eq!(id.repo(), "mlx-community/Qwen3-8B-4bit");
        assert_eq!(id.revision(), "def456");
    }

    #[test]
    fn rejects_missing_revision() {
        assert!(ModelId::parse("mlx-community/Qwen3-4B-Instruct-4bit").is_none());
    }

    #[test]
    fn new_id_is_unique() {
        assert_ne!(new_id(), new_id());
    }
}
