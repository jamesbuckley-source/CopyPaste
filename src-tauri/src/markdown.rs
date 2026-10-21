//! Markdown notes for a PKM vault: filenames, optional YAML front matter, and
//! the note body.
//!
//! The body is never reinterpreted. Formatting the clip is the vault's job; an
//! export only adds the metadata a note needs to be found again and a filename
//! that is safe on Windows.

use chrono::{DateTime, Local, SecondsFormat};
use serde::Serialize;

/// What an export wrote, and how many clips it had to leave out because they
/// have no Markdown body (images and file lists).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkdownExport {
    pub written: Vec<String>,
    pub skipped: usize,
}

/// Renders one note. `body` keeps its own text; only the line endings change.
pub fn render_note(
    body: &str,
    clip_id: i64,
    source_app: Option<&str>,
    tags: &[String],
    created_at: i64,
    front_matter: bool,
) -> String {
    let body = normalize_newlines(body);
    if !front_matter {
        return body;
    }

    let tags: Vec<String> = tags.iter().map(|tag| quote_yaml(tag)).collect();
    let mut note = String::from("---\n");
    note.push_str(&format!(
        "created: {}\n",
        quote_yaml(&local_time(created_at).to_rfc3339_opts(SecondsFormat::Secs, false))
    ));
    note.push_str(&format!(
        "source: {}\n",
        quote_yaml(source_app.unwrap_or_default())
    ));
    note.push_str(&format!("tags: [{}]\n", tags.join(", ")));
    note.push_str(&format!("clip_id: {}\n", quote_yaml(&clip_id.to_string())));
    note.push_str("---\n");
    note.push_str(&body);
    note
}

/// `YYYY-MM-DD <first line>.md`, or the same without the date when the setting
/// is off.
pub fn note_file_name(text: &str, created_at: i64, date_prefix: bool) -> String {
    let stem = file_stem(text);
    if date_prefix {
        format!("{} {stem}.md", local_time(created_at).format("%Y-%m-%d"))
    } else {
        format!("{stem}.md")
    }
}

/// CRLF and bare CR both become LF, so the note does not carry Windows endings
/// into a vault that may sync to other systems.
fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// A YAML double-quoted scalar. JSON-style escapes are valid YAML, which keeps
/// quotes, backslashes and newlines inside a value from breaking the block.
fn quote_yaml(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for c in value.chars() {
        match c {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            c if c.is_control() => quoted.push_str(&format!("\\u{:04x}", c as u32)),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

fn local_time(ms: i64) -> DateTime<Local> {
    match DateTime::from_timestamp_millis(ms) {
        Some(utc) => utc.with_timezone(&Local),
        None => DateTime::UNIX_EPOCH.with_timezone(&Local),
    }
}

/// Longest stem kept. The date prefix, a collision suffix and `.md` still fit
/// well inside Windows' path budget at this size.
const MAX_STEM_CHARS: usize = 80;

fn file_stem(text: &str) -> String {
    let first = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("clip");
    let cleaned = strip_markers(first);
    let cleaned = if is_only_markers(cleaned) {
        ""
    } else {
        cleaned
    };
    let mut stem: String = cleaned
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .take(MAX_STEM_CHARS)
        .collect();
    stem = stem.split_whitespace().collect::<Vec<_>>().join(" ");
    let stem = stem.trim().trim_end_matches(|c: char| c == '.' || c == ' ');
    if stem.is_empty() {
        return "clip".to_string();
    }
    if is_reserved_name(stem) {
        // Windows refuses CON, NUL, COM1 and friends even with an extension, so
        // nudge the name instead of failing the write.
        format!("_{stem}")
    } else {
        stem.to_string()
    }
}

/// Strips the decoration a heading, quote, list item or fence starts with.
fn strip_markers(mut line: &str) -> &str {
    loop {
        let before = line;
        line = line.trim_start();
        if let Some(rest) = strip_heading(line) {
            line = rest;
        } else if let Some(rest) = line.strip_prefix('>') {
            line = rest;
        } else if let Some(rest) = line
            .strip_prefix("- ")
            .or_else(|| line.strip_prefix("* "))
            .or_else(|| line.strip_prefix("+ "))
        {
            line = rest;
        } else if let Some(rest) = strip_ordered_item(line) {
            line = rest;
        } else if let Some(rest) = line
            .strip_prefix("[ ] ")
            .or_else(|| line.strip_prefix("[x] "))
            .or_else(|| line.strip_prefix("[X] "))
        {
            line = rest;
        } else if let Some(rest) = line
            .strip_prefix("```")
            .or_else(|| line.strip_prefix("~~~"))
        {
            line = rest;
        } else if let Some(rest) = line
            .strip_prefix("**")
            .or_else(|| line.strip_prefix("__"))
            .or_else(|| line.strip_prefix("~~"))
            .or_else(|| line.strip_prefix('*'))
            .or_else(|| line.strip_prefix('_'))
            .or_else(|| line.strip_prefix('`'))
        {
            line = rest;
        }
        if line == before {
            return line.trim_end();
        }
    }
}

fn strip_heading(line: &str) -> Option<&str> {
    let depth = line.chars().take_while(|c| *c == '#').count();
    if depth == 0 {
        return None;
    }
    let rest = &line[depth..];
    if rest.is_empty() {
        Some(rest)
    } else {
        rest.strip_prefix(' ')
    }
}

fn strip_ordered_item(line: &str) -> Option<&str> {
    let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
    // Three digits is plenty for a list; longer runs are more likely a year or
    // a number the sentence begins with.
    if !(1..=3).contains(&digits) {
        return None;
    }
    let rest = &line[digits..];
    rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") "))
}

/// A line that is only decoration (a horizontal rule, an empty fence) cannot
/// name a note.
fn is_only_markers(line: &str) -> bool {
    !line.is_empty()
        && line
            .chars()
            .all(|c| matches!(c, '-' | '=' | '*' | '_' | '~' | '`' | '#' | '>'))
}

fn is_reserved_name(stem: &str) -> bool {
    let base = stem.split('.').next().unwrap_or(stem).trim_end();
    let base = base.to_ascii_uppercase();
    matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && matches!(base.as_bytes()[3], b'1'..=b'9'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::unique_path;

    #[test]
    fn body_is_copied_with_newlines_normalised() {
        let body = "First line\r\nsecond line\rthird\n";
        assert_eq!(
            render_note(body, 7, None, &[], 0, false),
            "First line\nsecond line\nthird\n"
        );
    }

    #[test]
    fn front_matter_quotes_every_value() {
        let tags = vec![
            "a\"b".to_string(),
            "back\\slash".to_string(),
            "two\nlines".to_string(),
        ];
        let note = render_note(
            "# Heading\nbody",
            42,
            Some("Chrome \"incognito\""),
            &tags,
            0,
            true,
        );
        assert!(note.starts_with("---\ncreated: \""));
        assert!(note.contains("source: \"Chrome \\\"incognito\\\"\"\n"));
        assert!(note.contains("tags: [\"a\\\"b\", \"back\\\\slash\", \"two\\nlines\"]\n"));
        assert!(note.contains("clip_id: \"42\"\n---\n# Heading\nbody"));
    }

    #[test]
    fn yaml_quotes_escape_quotes_backslashes_and_control_characters() {
        assert_eq!(
            quote_yaml("a \"b\" \\ c\n\t"),
            "\"a \\\"b\\\" \\\\ c\\n\\t\""
        );
        assert_eq!(quote_yaml("\u{1}"), "\"\\u0001\"");
    }

    #[test]
    fn filenames_follow_the_date_setting() {
        let created = 1_700_000_000_000;
        let dated = note_file_name("# Release notes", created, true);
        assert!(
            dated.starts_with(&format!("{} ", local_time(created).format("%Y-%m-%d"))),
            "{dated}"
        );
        assert!(dated.ends_with(" Release notes.md"));
        assert_eq!(
            note_file_name("# Release notes", created, false),
            "Release notes.md"
        );
    }

    #[test]
    fn filenames_strip_markers_and_illegal_characters() {
        assert_eq!(note_file_name("## Plan: Q4/Q1", 0, false), "Plan Q4 Q1.md");
        assert_eq!(
            note_file_name("- [ ] write tests", 0, false),
            "write tests.md"
        );
        assert_eq!(note_file_name("1. First step", 0, false), "First step.md");
        assert_eq!(
            note_file_name("Trailing dots. . .", 0, false),
            "Trailing dots.md"
        );
        assert_eq!(note_file_name("\n\n", 0, false), "clip.md");
        assert_eq!(note_file_name("---", 0, false), "clip.md");
    }

    #[test]
    fn reserved_windows_names_are_avoided() {
        for name in ["CON", "con", "Prn", "AUX", "NUL", "COM1", "lpt9", "CON.txt"] {
            let file = note_file_name(name, 0, false);
            assert!(file.starts_with('_'), "{name} should be prefixed: {file}");
        }
        assert_eq!(note_file_name("CON", 0, false), "_CON.md");
        assert_eq!(note_file_name("CONSOLE", 0, false), "CONSOLE.md");
    }

    #[test]
    fn repeated_names_are_suffixed_rather_than_clobbered() {
        let dir = tempfile::tempdir().unwrap();
        let first = unique_path(dir.path(), "Note.md");
        std::fs::write(&first, "one").unwrap();
        let second = unique_path(dir.path(), "Note.md");
        std::fs::write(&second, "two").unwrap();
        assert_eq!(first.file_name().unwrap(), "Note.md");
        assert_eq!(second.file_name().unwrap(), "Note (2).md");
        assert_eq!(
            unique_path(dir.path(), "Note.md").file_name().unwrap(),
            "Note (3).md"
        );
    }
}
