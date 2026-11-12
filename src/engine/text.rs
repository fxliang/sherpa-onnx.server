use regex::Regex;
use std::sync::LazyLock;

static SPECIAL_TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<\|[^|>]+\|>").unwrap());

const NO_SPACE_BEFORE: &str = "，。！？、；：,.!?;:)]}）】》";
const NO_SPACE_AFTER: &str = "([{（【《";

fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{4E00}'..='\u{9FFF}'     // CJK Unified Ideographs
        | '\u{3400}'..='\u{4DBF}'   // CJK Extension A
        | '\u{20000}'..='\u{2A6DF}' // CJK Extension B
        | '\u{F900}'..='\u{FAFF}'   // CJK Compatibility Ideographs
        | '\u{3040}'..='\u{309F}'   // Hiragana
        | '\u{30A0}'..='\u{30FF}'   // Katakana
        | '\u{AC00}'..='\u{D7AF}'   // Hangul Syllables
    )
}

fn separator(prev: char, next: char) -> &'static str {
    if prev.is_whitespace() || next.is_whitespace() {
        return "";
    }
    if NO_SPACE_BEFORE.contains(next) {
        return "";
    }
    if NO_SPACE_AFTER.contains(prev) {
        return "";
    }
    if is_cjk(prev) || is_cjk(next) {
        return "";
    }
    " "
}

pub fn clean_text(text: &str) -> String {
    let text = SPECIAL_TOKEN_RE.replace_all(text, " ");
    let text = text.replace('\n', " ");
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.is_empty() {
        return String::new();
    }
    let mut result = String::from(tokens[0]);
    for &token in &tokens[1..] {
        let prev = result.chars().last().unwrap_or(' ');
        let next = token.chars().next().unwrap_or(' ');
        result.push_str(separator(prev, next));
        result.push_str(token);
    }
    result
}

pub fn join_text(base: &str, new_text: &str) -> String {
    let l = base.trim();
    let r = new_text.trim();
    if l.is_empty() {
        return r.to_string();
    }
    if r.is_empty() {
        return l.to_string();
    }
    let prev = l.chars().last().unwrap_or(' ');
    let next = r.chars().next().unwrap_or(' ');
    format!("{l}{}{r}", separator(prev, next))
}

pub fn join_segments(parts: &[String]) -> String {
    let mut result = String::new();
    for part in parts {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !result.is_empty() {
            let prev = result.chars().last().unwrap_or(' ');
            let next = trimmed.chars().next().unwrap_or(' ');
            result.push_str(separator(prev, next));
        }
        result.push_str(trimmed);
    }
    result
}
