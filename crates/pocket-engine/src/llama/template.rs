//! Chat templating for llama.cpp text generation.
//!
//! ENG-05 Chat Template Strategy:
//! 1. Embedded GGUF template: `llama-cpp-2` provides `LlamaModel::chat_template(None)`
//!    to extract the GGUF's embedded `tokenizer.chat_template` Jinja metadata and
//!    `LlamaModel::apply_chat_template(...)` to render it with `add_ass = true`.
//!    When available and successfully rendered, this embedded template is applied.
//! 2. Fallback template: If the GGUF does not specify an embedded chat template or if
//!    llama.cpp template application fails (e.g. unknown jinja syntax), we fall back
//!    to standard ChatML:
//!    `<|im_start|>role\ncontent<|im_end|>\n` per message in `messages`,
//!    ending with `<|im_start|>assistant\n` to prompt completion.

use llama_cpp_2::model::{LlamaChatMessage, LlamaModel};
use pocket_types::{ChatMessage, ContentPart, PocketError};

/// Extracts and concatenates all text fragments from a `ChatMessage`.
pub fn extract_message_text(msg: &ChatMessage) -> String {
    let mut text = String::new();
    for part in &msg.parts {
        if let ContentPart::Text { text: t } = part {
            text.push_str(t);
        }
    }
    text
}

/// Formats messages using the standard ChatML template:
/// `<|im_start|>role\ncontent<|im_end|>\n` ... `<|im_start|>assistant\n`
pub fn format_chatml(messages: &[ChatMessage]) -> String {
    let mut prompt = String::new();
    for msg in messages {
        let content = extract_message_text(msg);
        prompt.push_str("<|im_start|>");
        prompt.push_str(&msg.role);
        prompt.push('\n');
        prompt.push_str(&content);
        prompt.push_str("<|im_end|>\n");
    }
    prompt.push_str("<|im_start|>assistant\n");
    prompt
}

/// Applies either the model's embedded chat template or the ChatML fallback.
pub fn apply_model_template(
    model: &LlamaModel,
    messages: &[ChatMessage],
) -> Result<String, PocketError> {
    // Attempt embedded chat template from GGUF metadata
    if let Ok(tmpl) = model.chat_template(None) {
        let mut chat_messages = Vec::with_capacity(messages.len());
        let mut conversion_ok = true;

        for msg in messages {
            let content = extract_message_text(msg);
            match LlamaChatMessage::new(msg.role.clone(), content) {
                Ok(cm) => chat_messages.push(cm),
                Err(_) => {
                    // Null bytes in role or content prevented CString creation
                    conversion_ok = false;
                    break;
                }
            }
        }

        if conversion_ok {
            if let Ok(rendered) = model.apply_chat_template(&tmpl, &chat_messages, true) {
                return Ok(rendered);
            }
        }
    }

    // Fallback to standard ChatML format
    Ok(format_chatml(messages))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_chatml_single_user_message() {
        let messages = vec![ChatMessage {
            role: "user".into(),
            parts: vec![ContentPart::Text {
                text: "Hello, world!".into(),
            }],
        }];

        let expected = "<|im_start|>user\nHello, world!<|im_end|>\n<|im_start|>assistant\n";
        assert_eq!(format_chatml(&messages), expected);
    }

    #[test]
    fn format_chatml_multi_turn_conversation() {
        let messages = vec![
            ChatMessage {
                role: "system".into(),
                parts: vec![ContentPart::Text {
                    text: "You are a helpful assistant.".into(),
                }],
            },
            ChatMessage {
                role: "user".into(),
                parts: vec![ContentPart::Text {
                    text: "What is 2+2?".into(),
                }],
            },
            ChatMessage {
                role: "assistant".into(),
                parts: vec![ContentPart::Text { text: "4".into() }],
            },
            ChatMessage {
                role: "user".into(),
                parts: vec![ContentPart::Text {
                    text: "And plus 1?".into(),
                }],
            },
        ];

        let expected = concat!(
            "<|im_start|>system\nYou are a helpful assistant.<|im_end|>\n",
            "<|im_start|>user\nWhat is 2+2?<|im_end|>\n",
            "<|im_start|>assistant\n4<|im_end|>\n",
            "<|im_start|>user\nAnd plus 1?<|im_end|>\n",
            "<|im_start|>assistant\n"
        );
        assert_eq!(format_chatml(&messages), expected);
    }

    #[test]
    fn format_chatml_concatenates_multiple_text_parts() {
        let messages = vec![ChatMessage {
            role: "user".into(),
            parts: vec![
                ContentPart::Text {
                    text: "Hello ".into(),
                },
                ContentPart::Text {
                    text: "there!".into(),
                },
            ],
        }];

        let expected = "<|im_start|>user\nHello there!<|im_end|>\n<|im_start|>assistant\n";
        assert_eq!(format_chatml(&messages), expected);
    }

    #[test]
    fn format_chatml_empty_messages() {
        let messages: Vec<ChatMessage> = vec![];
        let expected = "<|im_start|>assistant\n";
        assert_eq!(format_chatml(&messages), expected);
    }
}
