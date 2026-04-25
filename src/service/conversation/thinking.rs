//! Thinking 标签提取（薄 wrapper）
//!
//! 真正的状态机实现仍在 [`super::stream`]（与 reducer 状态高耦合，Phase 4 不拆出独立状态机）；
//! 本模块对外暴露稳定 API：`ThinkingExtractor`、`extract_thinking_from_complete_text`。
//! Phase 7 优化时可把 stream.rs 的 thinking 部分剥离为独立模块。

#![allow(dead_code)]

/// 完整文本中提取 thinking 段：返回 (thinking_blocks, remaining_text)
///
/// 简化版：从首个 `<thinking>` 到末尾真正闭合 `</thinking>` 的文本作为 thinking；
/// 没有 thinking 标签则返回 (None, full)。
pub fn extract_thinking_from_complete_text(text: &str) -> (Option<String>, String) {
    let Some(start) = text.find("<thinking>") else {
        return (None, text.to_string());
    };
    let after = start + "<thinking>".len();
    let Some(end_rel) = text[after..].rfind("</thinking>") else {
        return (None, text.to_string());
    };
    let end_abs = after + end_rel;
    let thinking = text[after..end_abs].to_string();
    let mut remaining = String::with_capacity(text.len());
    remaining.push_str(&text[..start]);
    remaining.push_str(&text[end_abs + "</thinking>".len()..]);
    (Some(thinking), remaining)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_simple_thinking_block() {
        let text = "<thinking>plan</thinking>answer";
        let (thinking, rest) = extract_thinking_from_complete_text(text);
        assert_eq!(thinking.as_deref(), Some("plan"));
        assert_eq!(rest, "answer");
    }

    #[test]
    fn extract_no_thinking_returns_full_text() {
        let text = "just an answer";
        let (thinking, rest) = extract_thinking_from_complete_text(text);
        assert!(thinking.is_none());
        assert_eq!(rest, "just an answer");
    }

    #[test]
    fn extract_uses_last_closing_tag() {
        // 简化实现：取首个 <thinking> 到末尾的 </thinking>
        let text = "<thinking>step1 </thinking> mid </thinking> end";
        let (thinking, rest) = extract_thinking_from_complete_text(text);
        assert_eq!(
            thinking.as_deref(),
            Some("step1 </thinking> mid ")
        );
        assert_eq!(rest, " end");
    }
}
