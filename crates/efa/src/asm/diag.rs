//! Diagnostics of the `.efa` assembler: one error type, rendered like this
//! (design section 6; see `docs/efa-syntax.md`, "Diagnostics"):
//!
//! ```text
//! error: unknown register `acx`
//!   --> hello.efa:7:18
//!    |
//!  7 |     i64.add acc, acx, n
//!    |                  ^^^ did you mean `acc`?
//! ```

use super::ast::Span;
use std::fmt;

/// Width of a tab in the rendered source line.
const TAB_WIDTH: usize = 4;
/// Longest source excerpt (in chars) printed; longer lines are cut around the column.
const MAX_SHOWN: usize = 120;

/// One error: where it is, what is wrong, an optional note shown after the carets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub file: String,
    /// 1-based; 0 means "no location" (no snippet is printed).
    pub line: u32,
    /// 1-based, in chars.
    pub column: u32,
    /// Length in chars (at least one caret is printed).
    pub len: u32,
    pub message: String,
    pub note: Option<String>,
    /// The text of line `line` (without the line terminator); empty when `line == 0`.
    pub source_line: String,
}

impl Diagnostic {
    /// Builds a diagnostic and extracts the source line `span.line` from `src`.
    pub fn new(file: &str, src: &str, span: Span, message: impl Into<String>) -> Diagnostic {
        let text = if span.line == 0 {
            ""
        } else {
            src.split('\n')
                .nth(span.line as usize - 1)
                .map(|l| l.strip_suffix('\r').unwrap_or(l))
                .unwrap_or("")
        };
        Diagnostic::with_line(file, text, span, message)
    }

    /// Like [`Diagnostic::new`] with the source line already at hand (no scan of the source).
    pub fn with_line(
        file: &str,
        source_line: &str,
        span: Span,
        message: impl Into<String>,
    ) -> Diagnostic {
        Diagnostic {
            file: file.to_string(),
            line: span.line,
            column: span.column,
            len: span.len,
            message: message.into(),
            note: None,
            source_line: if span.line == 0 { String::new() } else { source_line.to_string() },
        }
    }

    /// A diagnostic with no location (`line == 0`), e.g. an error about the whole file.
    pub fn without_location(file: &str, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            file: file.to_string(),
            line: 0,
            column: 0,
            len: 0,
            message: message.into(),
            note: None,
            source_line: String::new(),
        }
    }

    /// Adds the note shown after the carets ("did you mean ...", "declared here", ...).
    pub fn with_note(mut self, note: impl Into<String>) -> Diagnostic {
        self.note = Some(note.into());
        self
    }

    pub fn span(&self) -> Span {
        Span { line: self.line, column: self.column, len: self.len }
    }
}

/// Visual width (tabs are `TAB_WIDTH`) of `count` chars of `chars` starting at char `start`;
/// chars past the end of the line count 1 each.
fn visual_width(chars: &[char], start: usize, count: usize) -> usize {
    let mut w = 0usize;
    for i in start..start.saturating_add(count) {
        match chars.get(i) {
            Some('\t') => w += TAB_WIDTH,
            _ => w += 1,
        }
        if w > 100_000 {
            break;
        }
    }
    w
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "error: {}", self.message)?;
        if self.line == 0 {
            write!(f, "\n  --> {}", self.file)?;
            if let Some(note) = &self.note {
                write!(f, "\n   = note: {}", note)?;
            }
            return Ok(());
        }
        let n = self.line.to_string().len();
        let gutter = " ".repeat(n + 2); // blank gutter, `|` follows
        write!(f, "\n{}--> {}:{}:{}", " ".repeat(n + 1), self.file, self.line, self.column)?;
        write!(f, "\n{}|", gutter)?;

        let chars: Vec<char> = self.source_line.trim_end().chars().collect();
        let start = (self.column.max(1) - 1) as usize;
        let (lo, hi) = if chars.len() > MAX_SHOWN {
            let c = start.min(chars.len());
            let lo = c.saturating_sub(MAX_SHOWN / 3);
            let hi = (lo + MAX_SHOWN).min(chars.len());
            (hi.saturating_sub(MAX_SHOWN).min(lo), hi)
        } else {
            (0, chars.len())
        };
        let truncated = lo > 0 || hi < chars.len();
        let window: String = chars.get(lo..hi).unwrap_or(&[]).iter().collect();
        let mut shown = window.replace('\t', &" ".repeat(TAB_WIDTH));
        if lo > 0 {
            shown.insert(0, '…');
        }
        if hi < chars.len() {
            shown.push('…');
        }
        if shown.is_empty() {
            write!(f, "\n {} |", self.line)?;
        } else {
            write!(f, "\n {} | {}", self.line, shown)?;
        }

        let lead = usize::from(lo > 0);
        let offset = lead + visual_width(&chars, lo, start.saturating_sub(lo));
        let want = self.len.max(1) as usize;
        let want = if truncated { want.min(hi.saturating_sub(start).max(1)) } else { want };
        let carets = visual_width(&chars, start, want);
        write!(f, "\n{}| {}{}", gutter, " ".repeat(offset), "^".repeat(carets))?;
        if let Some(note) = &self.note {
            write!(f, " {}", note)?;
        }
        Ok(())
    }
}

impl std::error::Error for Diagnostic {}

/// All the errors of one run. `Display` prints every diagnostic, separated by a blank line.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Diagnostics(pub Vec<Diagnostic>);

impl Diagnostics {
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Diagnostic> {
        self.0.iter()
    }
}

impl From<Vec<Diagnostic>> for Diagnostics {
    fn from(v: Vec<Diagnostic>) -> Diagnostics {
        Diagnostics(v)
    }
}

impl fmt::Display for Diagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, d) in self.0.iter().enumerate() {
            if i > 0 {
                write!(f, "\n\n")?;
            }
            write!(f, "{}", d)?;
        }
        Ok(())
    }
}

impl std::error::Error for Diagnostics {}

/// Levenshtein distance over chars.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = Vec::with_capacity(b.len() + 1);
        cur.push(i + 1);
        for (j, cb) in b.iter().enumerate() {
            let sub = prev.get(j).copied().unwrap_or(0) + usize::from(ca != cb);
            let del = prev.get(j + 1).copied().unwrap_or(0) + 1;
            let ins = cur.get(j).copied().unwrap_or(0) + 1;
            cur.push(sub.min(del).min(ins));
        }
        prev = cur;
    }
    prev.last().copied().unwrap_or(0)
}

/// The closest candidate to `name`, if one is within edit distance 2 (and the distance is
/// smaller than the length of `name`, so that one-letter names do not match everything).
/// Candidates equal to `name` are ignored. Ties go to the first candidate.
pub fn did_you_mean<I>(name: &str, candidates: I) -> Option<String>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let limit = 2usize.min(name.chars().count().saturating_sub(1));
    let mut best: Option<(usize, String)> = None;
    for c in candidates {
        let c = c.as_ref();
        if c == name {
            continue;
        }
        let d = edit_distance(name, c);
        if d <= limit && best.as_ref().map_or(true, |(bd, _)| d < *bd) {
            best = Some((d, c.to_string()));
        }
    }
    best.map(|(_, s)| s)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "import std.println(i64, i64)\n\nfn f(acc: i64, n: i64) {\n    i64.add acc, acx, n\n}\n";

    #[test]
    fn long_line_is_truncated_around_the_column() {
        let line = format!("{}X{}", "a".repeat(1000), "b".repeat(1000));
        let d = Diagnostic::new("a", &line, Span::new(1, 1001, 1), "m");
        let out = d.to_string();
        assert!(out.len() < 600, "{}", out.len());
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[3].contains('…') && lines[3].contains('X'));
        let x = lines[3].chars().position(|c| c == 'X').unwrap();
        let caret = lines[4].chars().position(|c| c == '^').unwrap();
        assert_eq!(x, caret);
        // A huge span is clipped too.
        let d = Diagnostic::new("a", &line, Span::new(1, 1001, 1000), "m");
        assert!(d.to_string().len() < 600);
    }

    #[test]
    fn render_with_note() {
        let d = Diagnostic::new("hello.efa", SRC, Span::new(4, 18, 3), "unknown register `acx`")
            .with_note("did you mean `acc`?");
        let want = "error: unknown register `acx`\n  --> hello.efa:4:18\n   |\n 4 |     i64.add acc, acx, n\n   |                  ^^^ did you mean `acc`?";
        assert_eq!(d.to_string(), want);
    }

    #[test]
    fn render_without_note() {
        let d = Diagnostic::new("a.efa", SRC, Span::new(3, 4, 1), "bad");
        let want = "error: bad\n  --> a.efa:3:4\n   |\n 3 | fn f(acc: i64, n: i64) {\n   |    ^";
        assert_eq!(d.to_string(), want);
    }

    #[test]
    fn render_two_digit_line() {
        let src = "\n".repeat(11) + "  x y\n";
        let d = Diagnostic::new("a.efa", &src, Span::new(12, 5, 1), "m");
        let want = "error: m\n   --> a.efa:12:5\n    |\n 12 |   x y\n    |     ^";
        assert_eq!(d.to_string(), want);
    }

    #[test]
    fn render_line_zero() {
        let d = Diagnostic::without_location("a.efa", "no `main` function");
        assert_eq!(d.to_string(), "error: no `main` function\n  --> a.efa");
        let d = d.with_note("add `export fn main`");
        assert_eq!(
            d.to_string(),
            "error: no `main` function\n  --> a.efa\n   = note: add `export fn main`"
        );
        // A span with line 0 also drops the snippet.
        let d = Diagnostic::new("a.efa", SRC, Span::none(), "x");
        assert_eq!(d.to_string(), "error: x\n  --> a.efa");
    }

    #[test]
    fn render_tab_in_source() {
        // `\t` is shown as four spaces and the carets are shifted by four.
        let src = "\ti64.add a, b, c\n";
        let d = Diagnostic::new("t.efa", src, Span::new(1, 10, 1), "m");
        let want = "error: m\n  --> t.efa:1:10\n   |\n 1 |     i64.add a, b, c\n   |             ^";
        assert_eq!(d.to_string(), want);
        // A tab inside the span widens the carets.
        let src = "a\tb\n";
        let d = Diagnostic::new("t.efa", src, Span::new(1, 1, 3), "m");
        let want = "error: m\n  --> t.efa:1:1\n   |\n 1 | a    b\n   | ^^^^^^";
        assert_eq!(d.to_string(), want);
    }

    #[test]
    fn render_crlf_and_missing_line() {
        let d = Diagnostic::new("c.efa", "ab\r\ncd\r\n", Span::new(1, 2, 1), "m");
        assert_eq!(d.source_line, "ab");
        let d = Diagnostic::new("c.efa", "ab\n", Span::new(9, 1, 1), "m");
        assert_eq!(d.to_string(), "error: m\n  --> c.efa:9:1\n   |\n 9 |\n   | ^");
    }

    #[test]
    fn zero_len_prints_one_caret() {
        let d = Diagnostic::new("a", "xy\n", Span::new(1, 3, 0), "eol");
        assert_eq!(d.to_string(), "error: eol\n  --> a:1:3\n   |\n 1 | xy\n   |   ^");
    }

    #[test]
    fn diagnostics_join() {
        let a = Diagnostic::without_location("f", "one");
        let b = Diagnostic::without_location("f", "two");
        let all = Diagnostics(vec![a, b]);
        assert_eq!(all.to_string(), "error: one\n  --> f\n\nerror: two\n  --> f");
        assert_eq!(all.len(), 2);
        assert!(!all.is_empty());
    }

    #[test]
    fn did_you_mean_basic() {
        assert_eq!(did_you_mean("acx", ["n", "acc", "t"]), Some("acc".to_string()));
        assert_eq!(did_you_mean("lenght", ["length", "x"]), Some("length".to_string()));
        assert_eq!(did_you_mean("i64.ad", ["i64.add", "i32.add"]), Some("i64.add".to_string()));
        assert_eq!(did_you_mean("zzzzz", ["acc", "n"]), None);
        // Distance 3 does not match.
        assert_eq!(did_you_mean("abcdef", ["abcxyz"]), None);
        // One-letter names do not match other one-letter names.
        assert_eq!(did_you_mean("x", ["n", "t"]), None);
        // Exact matches are ignored; ties go to the first; the closest wins.
        assert_eq!(did_you_mean("abc", ["abc"]), None);
        assert_eq!(did_you_mean("abc", ["abd", "abe"]), Some("abd".to_string()));
        assert_eq!(did_you_mean("abcd", ["abxx", "abcx"]), Some("abcx".to_string()));
        let owned = vec!["loop".to_string(), "done".to_string()];
        assert_eq!(did_you_mean("lop", owned.iter()), Some("loop".to_string()));
        assert_eq!(did_you_mean("", ["a"]), None);
    }

    #[test]
    fn edit_distance_values() {
        assert_eq!(edit_distance("", ""), 0);
        assert_eq!(edit_distance("abc", ""), 3);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("acx", "acc"), 1);
    }
}
