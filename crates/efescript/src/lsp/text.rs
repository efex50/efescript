//! Text helpers: LSP positions are 0-based lines and UTF-16 columns; the assembler's spans are
//! 1-based lines and char columns.

use crate::asm::ast::Span;
use serde_json::{json, Value};

/// The lines of `text` without their terminators.
pub fn lines(text: &str) -> Vec<&str> {
    text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect()
}

/// UTF-16 column of the char index `char_col` (past the end: one unit per missing char).
pub fn utf16_col(line: &str, char_col: usize) -> u32 {
    let mut n = 0u32;
    let mut seen = 0usize;
    for ch in line.chars() {
        if seen >= char_col {
            break;
        }
        n = n.saturating_add(ch.len_utf16() as u32);
        seen += 1;
    }
    n.saturating_add(char_col.saturating_sub(seen).min(u32::MAX as usize) as u32)
}

/// Char index of a UTF-16 column (clamped to the line; a column inside a surrogate pair
/// maps to the char before it).
pub fn char_col(line: &str, utf16: u32) -> usize {
    let mut n = 0u32;
    let mut c = 0usize;
    for ch in line.chars() {
        let w = ch.len_utf16() as u32;
        if n.saturating_add(w) > utf16 {
            break;
        }
        n += w;
        c += 1;
    }
    c
}

pub fn range_json(sl: u32, sc: u32, el: u32, ec: u32) -> Value {
    json!({"start": {"line": sl, "character": sc}, "end": {"line": el, "character": ec}})
}

/// The LSP range of a span on one line (a span without a location is 0:0-0:0).
pub fn span_range(lines: &[&str], span: Span) -> Value {
    if span.line == 0 {
        return range_json(0, 0, 0, 0);
    }
    let li = (span.line - 1) as usize;
    let line = lines.get(li).copied().unwrap_or("");
    let n = line.chars().count();
    let sc = (span.column.saturating_sub(1) as usize).min(n);
    let ec = (sc.saturating_add(span.len.max(1) as usize)).min(n);
    range_json(li as u32, utf16_col(line, sc), li as u32, utf16_col(line, ec))
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.'
}

/// True when the char index `col` is after a `;` comment start.
fn in_comment(chars: &[char], col: usize) -> bool {
    chars.iter().take(col).any(|&c| c == ';')
}

/// The word (`[alnum_.]+`) at or just before char column `col`: (start, end, text).
pub fn word_at(line: &str, col: usize) -> Option<(usize, usize, String)> {
    let chars: Vec<char> = line.chars().collect();
    let mut c = col.min(chars.len());
    if c < chars.len() && is_word(chars[c]) {
    } else if c > 0 && is_word(chars[c - 1]) {
        c -= 1;
    } else {
        return None;
    }
    let mut s = c;
    while s > 0 && is_word(chars[s - 1]) {
        s -= 1;
    }
    let mut e = c;
    while e < chars.len() && is_word(chars[e]) {
        e += 1;
    }
    if in_comment(&chars, s) {
        return None;
    }
    Some((s, e, chars[s..e].iter().collect()))
}

/// The word prefix ending at char column `col` (possibly empty): (start, prefix). `None` in a comment.
pub fn prefix_at(line: &str, col: usize) -> Option<(usize, String)> {
    let chars: Vec<char> = line.chars().collect();
    let c = col.min(chars.len());
    if in_comment(&chars, c) {
        return None;
    }
    let mut s = c;
    while s > 0 && is_word(chars[s - 1]) {
        s -= 1;
    }
    Some((s, chars[s..c].iter().collect()))
}

fn byte_offset(text: &str, line: u32, utf16: u32) -> usize {
    let mut off = 0usize;
    for (i, l) in text.split_inclusive('\n').enumerate() {
        if i == line as usize {
            let content = l.strip_suffix('\n').unwrap_or(l);
            let c = char_col(content, utf16);
            return off + content.char_indices().nth(c).map(|(b, _)| b).unwrap_or(content.len());
        }
        off += l.len();
    }
    text.len()
}

/// Replaces the range `(sl, sc)..(el, ec)` (LSP positions) of `text` with `new`.
pub fn apply_edit(text: &mut String, start: (u32, u32), end: (u32, u32), new: &str) {
    let a = byte_offset(text, start.0, start.1);
    let b = byte_offset(text, end.0, end.1);
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    text.replace_range(a..b, new);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_columns() {
        assert_eq!(utf16_col("a\u{1F600}b", 2), 3);
        assert_eq!(char_col("a\u{1F600}b", 3), 2);
        assert_eq!(char_col("a\u{1F600}b", 2), 1);
        assert_eq!(char_col("ab", 99), 2);
    }

    #[test]
    fn edits() {
        let mut t = "ab\ncd\n".to_string();
        apply_edit(&mut t, (0, 1), (1, 1), "X");
        assert_eq!(t, "aXd\n");
        apply_edit(&mut t, (9, 9), (9, 9), "!");
        assert_eq!(t, "aXd\n!");
    }
}
