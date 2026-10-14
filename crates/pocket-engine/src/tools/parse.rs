//! ENG-07: parsers for the two text-embedded tool-call formats llama.cpp
//! family models actually emit (native MLX tool-call events arrive as
//! structured `ToolCallDelta`s already and need no parsing - L3's job).
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedToolCall {
    pub name: String,
    /// Always a JSON object string, regardless of the source format's key
    /// name for the argument payload (`arguments` vs `parameters`).
    pub arguments_json: String,
}

#[derive(Deserialize)]
struct HermesCall {
    name: String,
    #[serde(default)]
    arguments: serde_json::Value,
}

#[derive(Deserialize)]
struct Llama3Call {
    name: String,
    #[serde(default)]
    parameters: serde_json::Value,
}

/// Hermes/Qwen format: one or more `<tool_call>{json}</tool_call>` blocks
/// anywhere in the text. Malformed JSON inside a block is skipped, not
/// fatal - a model occasionally emits a broken call alongside good ones.
pub fn parse_tool_calls_hermes(text: &str) -> Vec<ParsedToolCall> {
    const OPEN: &str = "<tool_call>";
    const CLOSE: &str = "</tool_call>";
    let mut calls = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        let after_open = &rest[start + OPEN.len()..];
        let Some(end) = after_open.find(CLOSE) else { break };
        let body = after_open[..end].trim();
        if let Ok(parsed) = serde_json::from_str::<HermesCall>(body) {
            calls.push(ParsedToolCall {
                name: parsed.name,
                arguments_json: parsed.arguments.to_string(),
            });
        }
        rest = &after_open[end + CLOSE.len()..];
    }
    calls
}

/// Llama 3.x built-in tool calling: the entire completion (after trimming)
/// is a single JSON object `{"name": "...", "parameters": {...}}` - no
/// wrapping tags.
pub fn parse_tool_calls_llama3(text: &str) -> Vec<ParsedToolCall> {
    let trimmed = text.trim();
    match serde_json::from_str::<Llama3Call>(trimmed) {
        Ok(parsed) => vec![ParsedToolCall {
            name: parsed.name,
            arguments_json: parsed.parameters.to_string(),
        }],
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_single_hermes_tool_call() {
        let text = r#"<tool_call>{"name": "calculator", "arguments": {"expression": "2+2"}}</tool_call>"#;
        let calls = parse_tool_calls_hermes(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "calculator");
        let args: serde_json::Value = serde_json::from_str(&calls[0].arguments_json).unwrap();
        assert_eq!(args["expression"], "2+2");
    }

    #[test]
    fn parses_multiple_hermes_tool_calls_with_surrounding_text() {
        let text = "Let me check.\n<tool_call>{\"name\": \"datetime\", \"arguments\": {}}</tool_call>\nand\n<tool_call>{\"name\": \"calculator\", \"arguments\": {\"expression\": \"1+1\"}}</tool_call>";
        let calls = parse_tool_calls_hermes(text);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "datetime");
        assert_eq!(calls[1].name, "calculator");
    }

    #[test]
    fn skips_a_malformed_hermes_block_but_keeps_good_ones() {
        let text = r#"<tool_call>not json</tool_call><tool_call>{"name": "calculator", "arguments": {"expression": "3*3"}}</tool_call>"#;
        let calls = parse_tool_calls_hermes(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "calculator");
    }

    #[test]
    fn returns_empty_when_no_hermes_tags_present() {
        assert!(parse_tool_calls_hermes("just a plain answer").is_empty());
    }

    #[test]
    fn parses_llama3_style_json_tool_call() {
        let text = r#"  {"name": "calculator", "parameters": {"expression": "5-2"}}  "#;
        let calls = parse_tool_calls_llama3(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "calculator");
        let args: serde_json::Value = serde_json::from_str(&calls[0].arguments_json).unwrap();
        assert_eq!(args["expression"], "5-2");
    }

    #[test]
    fn llama3_parser_returns_empty_for_plain_text() {
        assert!(parse_tool_calls_llama3("just a plain answer").is_empty());
    }
}
