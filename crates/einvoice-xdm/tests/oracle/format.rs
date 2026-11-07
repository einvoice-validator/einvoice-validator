//! The probe files of the Saxon oracle, shared by the `saxon-oracle` example (which
//! runs Saxon) and the `oracle` test (which runs this crate).
//!
//! `cases.tsv` and `generated.tsv`: one case per line, tab-separated:
//! `id  op  type:lexical ...`. Empty lines and lines starting with `#` are ignored.
//! `expected.tsv`: `id  result ...` where each result column is `type:value` for one
//! item, `error:CODE`, or `empty` for the empty sequence.
//!
//! In lexical forms and values, `\t`, `\n`, `\r`, `\\` and `\u{HEX}` stand for the
//! characters; every other character, including any other backslash, stands for
//! itself (a regular expression escape such as `\n` is written `\\n`).

use std::fmt::Write as _;

/// One probe case.
#[derive(Debug, Clone)]
pub(crate) struct Case {
    pub(crate) id: String,
    pub(crate) op: String,
    pub(crate) args: Vec<Arg>,
}

/// One operand: the way to build it (`decimal`, `untyped`, ...) and its lexical form.
#[derive(Debug, Clone)]
pub(crate) struct Arg {
    pub(crate) kind: String,
    pub(crate) lexical: String,
}

/// Escapes a string for a TSV column.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c.is_control()
                || matches!(u32::from(c), 0x80..=0x9F | 0xA0 | 0x2000..=0x200F | 0x2028..=0x202F | 0xFEFF) =>
            {
                let _ = write!(out, "\\u{{{:X}}}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out
}

/// Reverses [`escape`].
pub(crate) fn unescape(text: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') | None => out.push('\\'),
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('u') => {
                if chars.next() != Some('{') {
                    return Err(format!("bad \\u escape in {text:?}"));
                }
                let hex: String = chars.by_ref().take_while(|&c| c != '}').collect();
                let code = u32::from_str_radix(&hex, 16)
                    .map_err(|e| format!("bad \\u escape in {text:?}: {e}"))?;
                out.push(
                    char::from_u32(code).ok_or_else(|| format!("bad code point in {text:?}"))?,
                );
            }
            // Any other backslash is literal, so that regular expressions read
            // naturally (`\s`, `\d`, `\.`).
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    Ok(out)
}

/// Reads a case file.
pub(crate) fn read_cases(text: &str) -> Result<Vec<Case>, String> {
    let mut cases = Vec::new();
    for (number, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut columns = line.split('\t');
        let id = columns.next().unwrap_or_default().to_owned();
        let op = columns
            .next()
            .ok_or_else(|| format!("line {}: no operation", number + 1))?
            .to_owned();
        let mut args = Vec::new();
        for column in columns {
            let (kind, lexical) = column
                .split_once(':')
                .ok_or_else(|| format!("line {}: operand without type: {column:?}", number + 1))?;
            args.push(Arg {
                kind: kind.to_owned(),
                lexical: unescape(lexical).map_err(|e| format!("line {}: {e}", number + 1))?,
            });
        }
        cases.push(Case { id, op, args });
    }
    Ok(cases)
}

/// Reads an expected-results file: the id and the result columns, joined by tabs.
pub(crate) fn read_expected(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| match line.split_once('\t') {
            Some((id, result)) => (id.to_owned(), result.to_owned()),
            None => (line.to_owned(), String::new()),
        })
        .collect()
}
