//! ENG-06: streams text apart from `<think>...</think>` content so an
//! engine's token-by-token output can route the former to
//! `GenerateDelta.text` and the latter to `GenerateDelta.reasoning`, even
//! when the opening/closing tag itself is split across chunk boundaries.
const OPEN_TAG: &str = "<think>";
const CLOSE_TAG: &str = "</think>";

#[derive(Default)]
pub struct ReasoningSplitter {
    buffer: String,
    in_think: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Split {
    pub text: String,
    pub reasoning: String,
}

impl ReasoningSplitter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed the next chunk of raw model output; returns whatever can be
    /// emitted now (text/reasoning accumulated so far from this call,
    /// possibly spanning several open/close tags within one chunk).
    pub fn push(&mut self, chunk: &str) -> Split {
        self.buffer.push_str(chunk);
        let mut split = Split::default();
        loop {
            let tag = if self.in_think { CLOSE_TAG } else { OPEN_TAG };
            if let Some(pos) = self.buffer.find(tag) {
                let before = self.buffer[..pos].to_string();
                let after = self.buffer[pos + tag.len()..].to_string();
                self.emit(&before, &mut split);
                self.in_think = !self.in_think;
                self.buffer = after;
                continue;
            }

            let holdback = longest_suffix_prefix_overlap(&self.buffer, tag);
            let emit_len = self.buffer.len() - holdback;
            let emitted = self.buffer[..emit_len].to_string();
            self.emit(&emitted, &mut split);
            self.buffer = self.buffer[emit_len..].to_string();
            break;
        }
        split
    }

    /// Call once generation is done: any buffered partial tag never
    /// completed is plain content, not a reasoning marker.
    pub fn flush(&mut self) -> Split {
        let mut split = Split::default();
        let remaining = std::mem::take(&mut self.buffer);
        self.emit(&remaining, &mut split);
        split
    }

    fn emit(&self, text: &str, split: &mut Split) {
        if text.is_empty() {
            return;
        }
        if self.in_think {
            split.reasoning.push_str(text);
        } else {
            split.text.push_str(text);
        }
    }
}

/// The longest suffix of `buffer` that is also a (non-full) prefix of
/// `tag` - the part we must hold back since the next chunk could complete
/// the tag.
fn longest_suffix_prefix_overlap(buffer: &str, tag: &str) -> usize {
    let max_len = buffer.len().min(tag.len() - 1);
    for len in (1..=max_len).rev() {
        if buffer.ends_with(&tag[..len]) {
            return len;
        }
    }
    0
}

/// A§4.4: `ReasoningMode::Off` suppresses thinking either by appending a
/// `/no_think` suffix (Qwen3-family chat templates) or by setting the
/// `enable_thinking` template variable to `false` (any template that
/// supports it) - which one applies depends on the model's `family`.
#[derive(Debug, PartialEq, Eq)]
pub enum NoThinkStrategy {
    AppendSuffix(String),
    TemplateVar(String, bool),
}

pub fn no_think_strategy_for_family(family: &str) -> NoThinkStrategy {
    if family.to_lowercase().contains("qwen3") {
        NoThinkStrategy::AppendSuffix("/no_think".into())
    } else {
        NoThinkStrategy::TemplateVar("enable_thinking".into(), false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_with_no_tags_passes_through_untouched() {
        let mut s = ReasoningSplitter::new();
        let split = s.push("hello world");
        assert_eq!(split.text, "hello world");
        assert_eq!(split.reasoning, "");
    }

    #[test]
    fn routes_content_inside_think_tags_to_reasoning() {
        let mut s = ReasoningSplitter::new();
        let split = s.push("before <think>pondering</think> after");
        assert_eq!(split.text, "before  after");
        assert_eq!(split.reasoning, "pondering");
    }

    #[test]
    fn splits_opening_tag_across_chunk_boundaries() {
        let mut s = ReasoningSplitter::new();
        let mut text = String::new();
        let mut reasoning = String::new();
        for chunk in ["before <thi", "nk>pondering</thi", "nk> after"] {
            let split = s.push(chunk);
            text.push_str(&split.text);
            reasoning.push_str(&split.reasoning);
        }
        assert_eq!(text, "before  after");
        assert_eq!(reasoning, "pondering");
    }

    #[test]
    fn splits_one_token_at_a_time() {
        let mut s = ReasoningSplitter::new();
        let mut text = String::new();
        let mut reasoning = String::new();
        let full = "a<think>b</think>c";
        for ch in full.chars() {
            let split = s.push(&ch.to_string());
            text.push_str(&split.text);
            reasoning.push_str(&split.reasoning);
        }
        let end = s.flush();
        text.push_str(&end.text);
        reasoning.push_str(&end.reasoning);
        assert_eq!(text, "ac");
        assert_eq!(reasoning, "b");
    }

    #[test]
    fn handles_multiple_think_blocks() {
        let mut s = ReasoningSplitter::new();
        let split = s.push("a<think>r1</think>b<think>r2</think>c");
        assert_eq!(split.text, "abc");
        assert_eq!(split.reasoning, "r1r2");
    }

    #[test]
    fn an_unclosed_think_tag_at_stream_end_is_flushed_as_reasoning() {
        let mut s = ReasoningSplitter::new();
        let split = s.push("a<think>unfinished");
        assert_eq!(split.text, "a");
        assert_eq!(split.reasoning, "unfinished");
        let end = s.flush();
        assert_eq!(end.text, "");
        assert_eq!(end.reasoning, "");
    }

    #[test]
    fn a_lone_angle_bracket_that_never_becomes_a_tag_is_still_emitted() {
        let mut s = ReasoningSplitter::new();
        let split = s.push("1 < 2 and 3 > 2");
        let end = s.flush();
        assert_eq!(format!("{}{}", split.text, end.text), "1 < 2 and 3 > 2");
    }

    #[test]
    fn qwen3_family_uses_the_no_think_suffix() {
        assert_eq!(no_think_strategy_for_family("Qwen3-4B"), NoThinkStrategy::AppendSuffix("/no_think".into()));
    }

    #[test]
    fn other_families_use_the_template_variable() {
        assert_eq!(
            no_think_strategy_for_family("Llama-3.1"),
            NoThinkStrategy::TemplateVar("enable_thinking".into(), false)
        );
    }
}
