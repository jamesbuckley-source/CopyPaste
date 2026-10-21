//! Color codes copied as text.
//!
//! A clip is treated as a color only when its *entire* payload is a color, so a
//! document that happens to mention `#ff0000` is not swallowed by the swatch view.

/// Normalises a whole clipboard payload to `#rrggbb`, or `None` when it is not a
/// single color value.
pub fn parse(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.lines().count() != 1 {
        return None;
    }
    let lowered = trimmed.to_ascii_lowercase();
    parse_hex(&lowered).or_else(|| parse_rgb(&lowered))
}

fn parse_hex(value: &str) -> Option<String> {
    // `#rrggbb` is the canonical form, `0xrrggbb` shows up in code, and design
    // tools frequently hand over the bare digits.
    let (digits, explicit) = if let Some(digits) = value.strip_prefix('#') {
        (digits, true)
    } else if let Some(digits) = value.strip_prefix("0x") {
        (digits, true)
    } else {
        (value, false)
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    if !explicit {
        // A bare value is only a color when it looks like one: hex digits that
        // include both a number and a letter. Without that test "facade" and
        // "decade" would be colors, and so would the number 123456.
        let has_digit = digits.chars().any(|c| c.is_ascii_digit());
        let has_letter = digits.chars().any(|c| c.is_ascii_alphabetic());
        if !has_digit || !has_letter {
            return None;
        }
        if !matches!(digits.len(), 3 | 6 | 8) {
            return None;
        }
    }
    match digits.len() {
        // #rgb and #rgba expand each digit, which is what CSS does too.
        3 | 4 => {
            let expanded: String = digits.chars().take(3).flat_map(|c| [c, c]).collect();
            Some(format!("#{expanded}"))
        }
        // Alpha is accepted but dropped: the sample is painted opaque.
        6 | 8 => Some(format!("#{}", &digits[..6])),
        _ => None,
    }
}

fn parse_rgb(value: &str) -> Option<String> {
    let inner = value
        .strip_prefix("rgb(")
        .or_else(|| value.strip_prefix("rgba("))?
        .strip_suffix(')')?;

    // CSS Color 4 accepts spaces instead of commas and a `/` before the alpha, so
    // `rgb(255 136 0)`, `rgb(255, 136, 0)` and `rgb(255 136 0 / 50%)` all work.
    let (channels_text, alpha_text) = match inner.split_once('/') {
        Some((channels, alpha)) => (channels, Some(alpha.trim().to_string())),
        None => (inner, None),
    };
    let split = |text: &str| -> Vec<String> {
        if text.contains(',') {
            text.split(',')
                .map(|part| part.trim().to_string())
                .filter(|part| !part.is_empty())
                .collect()
        } else {
            text.split_whitespace().map(str::to_string).collect()
        }
    };

    let mut parts = split(channels_text);
    let alpha = match alpha_text {
        Some(alpha) => Some(alpha),
        // The comma form carries the alpha as a fourth channel.
        None if parts.len() == 4 => parts.pop(),
        None => None,
    };
    if parts.len() != 3 {
        return None;
    }

    let mut channels = [0u8; 3];
    for (slot, part) in channels.iter_mut().zip(&parts) {
        *slot = parse_channel(part)?;
    }
    if let Some(alpha) = alpha {
        parse_alpha(&alpha)?;
    }
    Some(format!(
        "#{:02x}{:02x}{:02x}",
        channels[0], channels[1], channels[2]
    ))
}

fn parse_channel(part: &str) -> Option<u8> {
    if let Some(percent) = part.strip_suffix('%') {
        let value: f64 = percent.trim().parse().ok()?;
        if !(0.0..=100.0).contains(&value) {
            return None;
        }
        return Some((value / 100.0 * 255.0).round() as u8);
    }
    let value: i64 = part.parse().ok()?;
    (0..=255).contains(&value).then_some(value as u8)
}

fn parse_alpha(part: &str) -> Option<()> {
    if let Some(percent) = part.strip_suffix('%') {
        let value: f64 = percent.trim().parse().ok()?;
        return (0.0..=100.0).contains(&value).then_some(());
    }
    let value: f64 = part.parse().ok()?;
    (0.0..=1.0).contains(&value).then_some(())
}

/// Shape of a text payload the UI can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    Color,
    Url,
    Email,
    Path,
    Json,
    Markdown,
}

impl ContentKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            ContentKind::Color => "color",
            ContentKind::Url => "url",
            ContentKind::Email => "email",
            ContentKind::Path => "path",
            ContentKind::Json => "json",
            ContentKind::Markdown => "markdown",
        }
    }
}

/// Classifies a whole text payload, so the UI can offer matching actions.
///
/// The single-value shapes only describe a one-line payload, so they are only
/// consulted then. Markdown is the one shape that describes structured multi-line
/// text, which is why it is the fallback: a document that happens to contain a
/// heading is Markdown, while `#ff0000` alone stays a color.
pub fn classify(text: &str) -> Option<ContentKind> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.lines().count() == 1 {
        if parse(trimmed).is_some() {
            return Some(ContentKind::Color);
        }
        if is_url(trimmed) {
            return Some(ContentKind::Url);
        }
        if is_email(trimmed) {
            return Some(ContentKind::Email);
        }
        if is_path(trimmed) {
            return Some(ContentKind::Path);
        }
        if is_json(trimmed) {
            return Some(ContentKind::Json);
        }
    }
    is_markdown(trimmed).then_some(ContentKind::Markdown)
}

fn is_url(text: &str) -> bool {
    (text.starts_with("http://") || text.starts_with("https://"))
        && !text.chars().any(char::is_whitespace)
        && text.len() <= 2048
}

fn is_email(text: &str) -> bool {
    if text.len() > 254 || text.chars().any(char::is_whitespace) {
        return false;
    }
    let Some((local, domain)) = text.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains('@')
}

fn is_path(text: &str) -> bool {
    if text.chars().any(char::is_whitespace) || text.len() > 4096 {
        return false;
    }
    let bytes = text.as_bytes();
    // Drive-letter path (C:\...) or UNC share (\\server\share).
    (bytes.len() > 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/'))
        || text.starts_with("\\\\")
}

fn is_json(text: &str) -> bool {
    let first = text.as_bytes().first().copied();
    if !matches!(first, Some(b'{') | Some(b'[')) {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(text).is_ok()
}

/// Detects Markdown formatting clearly enough to be worth a badge.
///
/// The bar is deliberately high: the badge replaces the clip's kind in the list,
/// so a wrong one is worse than none. Only syntax that essentially cannot occur in
/// ordinary prose counts, and a stray `*`, `#` or `-` inside a sentence never
/// does.
fn is_markdown(text: &str) -> bool {
    let mut fences = 0usize;
    for line in text.lines() {
        let line = line.trim_start();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("```") || line.starts_with("~~~") {
            fences += 1;
            continue;
        }
        if is_heading(line) || is_block_quote(line) || is_list_item(line) || is_table_row(line) {
            return true;
        }
    }
    // A lone fence is more likely a pasted fragment than a code block, so the
    // block has to be closed again before it counts.
    if fences >= 2 {
        return true;
    }
    has_link(text) || has_paired_span(text, "*") || has_code_span(text) || has_underscore_span(text)
}

/// `#` to `######` followed by a space, at the start of a line.
fn is_heading(line: &str) -> bool {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    (1..=6).contains(&hashes) && line[hashes..].starts_with(' ')
}

fn is_block_quote(line: &str) -> bool {
    line.starts_with("> ")
}

/// `- item`, `* item`, `+ item`, `1. item` or `1) item` at the start of a line.
fn is_list_item(line: &str) -> bool {
    if ["- ", "* ", "+ "]
        .iter()
        .any(|marker| line.strip_prefix(marker).is_some_and(|rest| !rest.trim().is_empty()))
    {
        return true;
    }
    // One or two digits only: "2024. A year starts a sentence" is prose, not a list.
    let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
    if !(1..=2).contains(&digits) {
        return false;
    }
    let rest = &line[digits..];
    [". ", ") "]
        .iter()
        .any(|marker| rest.strip_prefix(marker).is_some_and(|rest| !rest.trim().is_empty()))
}

/// A table row: pipes at both ends with at least one cell between them.
fn is_table_row(line: &str) -> bool {
    line.starts_with('|') && line.ends_with('|') && line.matches('|').count() >= 3
}

/// `[label](target)` or `![alt](target)`, with a non-empty label and a target that
/// is not just prose in brackets.
fn has_link(text: &str) -> bool {
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let after_open = &rest[open + 1..];
        let Some(close) = after_open.find(']') else {
            return false;
        };
        let label = &after_open[..close];
        let after_label = &after_open[close + 1..];
        if let Some(after_paren) = after_label.strip_prefix('(')
            && let Some(end) = after_paren.find(')')
        {
            let target = &after_paren[..end];
            if !label.trim().is_empty()
                && !label.contains('[')
                && !target.trim().is_empty()
                && !target.chars().any(char::is_whitespace)
            {
                return true;
            }
        }
        rest = after_label;
    }
    false
}

/// True when `delimiter` appears twice around a run of non-space text.
///
/// A single `*` covers both `*italic*` and `**bold**`, since the inner run is
/// delimited either way. The whitespace test keeps arithmetic like `5 ** 2` and
/// Markdown's own `** bold **` (which renders literally) from counting.
fn has_paired_span(text: &str, delimiter: &str) -> bool {
    let mut from = 0;
    while let Some(offset) = text[from..].find(delimiter) {
        let content_start = from + offset + delimiter.len();
        let Some(close) = text[content_start..].find(delimiter) else {
            return false;
        };
        let content = &text[content_start..content_start + close];
        if !content.is_empty() && content.trim() == content {
            return true;
        }
        // An empty run means the delimiters were touching (`**`), so the second
        // one may still open a real span; skipping both would lose it.
        from = if content.is_empty() {
            content_start
        } else {
            content_start + close + delimiter.len()
        };
    }
    false
}

/// A single backtick pair; fenced blocks are handled separately.
fn has_code_span(text: &str) -> bool {
    has_paired_span(text, "`")
}

/// `_italic_` counts only when the underscores are delimiters, so identifiers such
/// as `snake_case_names` are left alone.
fn has_underscore_span(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'_' {
            index += 1;
            continue;
        }
        // An opening delimiter follows punctuation or a space, never a word.
        if index > 0 && is_word_byte(bytes[index - 1]) {
            index += 1;
            continue;
        }
        let content_start = index + 1;
        let Some(offset) = text[content_start..].find('_') else {
            return false;
        };
        let close = content_start + offset;
        let content = &text[content_start..close];
        let closed = close + 1 == bytes.len() || !is_word_byte(bytes[close + 1]);
        if closed
            && !content.is_empty()
            && !content.chars().any(char::is_whitespace)
            && !content.starts_with('_')
        {
            return true;
        }
        index = close + 1;
    }
    false
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_hex_in_every_common_form() {
        assert_eq!(parse("#ff8800").as_deref(), Some("#ff8800"));
        assert_eq!(parse("  #FF8800  ").as_deref(), Some("#ff8800"));
        assert_eq!(parse("#f80").as_deref(), Some("#ff8800"));
        assert_eq!(parse("#f80f").as_deref(), Some("#ff8800"), "alpha is dropped");
        assert_eq!(parse("#ff880080").as_deref(), Some("#ff8800"));
    }

    #[test]
    fn recognises_rgb_and_rgba() {
        assert_eq!(parse("rgb(255, 136, 0)").as_deref(), Some("#ff8800"));
        assert_eq!(parse("RGB(255,136,0)").as_deref(), Some("#ff8800"));
        assert_eq!(parse("rgba(255, 136, 0, 0.5)").as_deref(), Some("#ff8800"));
        assert_eq!(parse("rgb(100%, 53.3%, 0%)").as_deref(), Some("#ff8800"));
    }

    #[test]
    fn ignores_prose_and_invalid_values() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("the color is #ff0000"), None, "not the whole payload");
        assert_eq!(parse("#ff0000 on a red background"), None);
        assert_eq!(parse("#ff0000\n#00ff00"), None, "multi-line payloads are text");
        assert_eq!(parse("#12345"), None);
        assert_eq!(parse("rgb(256, 0, 0)"), None);
        assert_eq!(parse("rgb(1,2,3,4,5)"), None);
        assert_eq!(parse("rgb(1,2,3, 2)"), None);
        assert_eq!(parse("#gggggg"), None);
        // Bare hex is only taken when it looks like a color rather than a word.
        assert_eq!(parse("facade"), None, "six hex letters that spell a word");
        assert_eq!(parse("decade"), None);
        assert_eq!(parse("beaded"), None);
        assert_eq!(parse("123456"), None, "a bare number is not a color");
        assert_eq!(parse("face"), None, "four bare digits are ambiguous");
    }

    #[test]
    fn accepts_bare_and_prefixed_hex_from_design_tools() {
        assert_eq!(parse("ff8800").as_deref(), Some("#ff8800"));
        assert_eq!(parse("FF8800").as_deref(), Some("#ff8800"));
        assert_eq!(parse("f80").as_deref(), Some("#ff8800"));
        assert_eq!(parse("ff8800ff").as_deref(), Some("#ff8800"));
        assert_eq!(parse("0xFF8800").as_deref(), Some("#ff8800"));
        assert_eq!(parse("0xffffff").as_deref(), Some("#ffffff"));
        // The digit test rejects pure-letter values, which are usually words…
        assert_eq!(parse("ffffff"), None);
        // …but accepts the famous ones that carry a number.
        assert_eq!(parse("c0ffee").as_deref(), Some("#c0ffee"));
    }

    #[test]
    fn accepts_css_color_4_rgb_syntax() {
        assert_eq!(parse("rgb(255 136 0)").as_deref(), Some("#ff8800"));
        assert_eq!(parse("rgb(255 136 0 / 0.5)").as_deref(), Some("#ff8800"));
        assert_eq!(parse("rgb(255 136 0 / 50%)").as_deref(), Some("#ff8800"));
        assert_eq!(parse("rgba(255 136 0 / 1)").as_deref(), Some("#ff8800"));
        assert_eq!(parse("rgb(255, 136, 0 / 50%)").as_deref(), Some("#ff8800"));
        assert_eq!(parse("rgb(255 136)"), None, "two channels are not a color");
        assert_eq!(parse("rgb(255 136 0 / 2)"), None, "alpha above one");
    }

    #[test]
    fn classifies_actionable_payloads() {
        assert_eq!(classify("#ff8800"), Some(ContentKind::Color));
        assert_eq!(classify("https://example.com/a?b=1"), Some(ContentKind::Url));
        assert_eq!(classify("someone@example.com"), Some(ContentKind::Email));
        assert_eq!(classify("C:\\temp\\file.txt"), Some(ContentKind::Path));
        assert_eq!(classify("{\"a\":1}"), Some(ContentKind::Json));
        assert_eq!(classify("[1, 2, 3]"), Some(ContentKind::Json));
        assert_eq!(classify("https://example.com has spaces"), None);
        assert_eq!(classify("not an email"), None);
        assert_eq!(classify("C:\\temp file.txt"), None);
        assert_eq!(classify("{\"broken\": }"), None);
        assert_eq!(classify("123"), None, "a bare number is not useful JSON");
        assert_eq!(classify("line one\nline two"), None);
    }

    #[test]
    fn recognises_markdown_formatting() {
        assert_eq!(classify("# Release notes"), Some(ContentKind::Markdown));
        assert_eq!(classify("### Details"), Some(ContentKind::Markdown));
        assert_eq!(
            classify("Shopping\n\n- milk\n- bread"),
            Some(ContentKind::Markdown)
        );
        assert_eq!(
            classify("Steps:\n1. build\n2. test"),
            Some(ContentKind::Markdown)
        );
        assert_eq!(classify("- [ ] write the tests"), Some(ContentKind::Markdown));
        assert_eq!(classify("> quoted line"), Some(ContentKind::Markdown));
        assert_eq!(
            classify("| Name | Value |\n| --- | --- |\n| a | 1 |"),
            Some(ContentKind::Markdown)
        );
        assert_eq!(
            classify("```rust\nlet x = 1;\n```"),
            Some(ContentKind::Markdown)
        );
        assert_eq!(
            classify("Read [the docs](https://example.com/a) first"),
            Some(ContentKind::Markdown)
        );
        assert_eq!(classify("![shot](shot.png)"), Some(ContentKind::Markdown));
        assert_eq!(classify("make this **bold** please"), Some(ContentKind::Markdown));
        assert_eq!(classify("an _italic_ word"), Some(ContentKind::Markdown));
        assert_eq!(classify("an *italic* word"), Some(ContentKind::Markdown));
        assert_eq!(classify("run `cargo test` now"), Some(ContentKind::Markdown));
    }

    #[test]
    fn plain_prose_is_not_markdown() {
        assert_eq!(
            classify("This is an ordinary sentence. It has 2.5 numbers and a # sign."),
            None
        );
        assert_eq!(classify("#hashtag is not a heading"), None);
        assert_eq!(classify("a > b is a comparison"), None);
        assert_eq!(classify("state-of-the-art"), None);
        assert_eq!(classify("2024. A year at the start of a sentence."), None);
        assert_eq!(classify("5 ** 2 is 25"), None, "arithmetic is not bold");
        assert_eq!(classify("2 * 3 * 4"), None, "arithmetic is not italic");
        assert_eq!(classify("** spaced **"), None);
        assert_eq!(classify("**unclosed bold"), None);
        assert_eq!(classify("call snake_case_names here"), None);
        assert_eq!(classify("a|b|c"), None, "no leading pipe");
        assert_eq!(classify("```\nunclosed fence"), None);
    }

    #[test]
    fn single_value_shapes_still_win_over_markdown() {
        assert_eq!(classify("#ff8800"), Some(ContentKind::Color));
        assert_eq!(
            classify("https://example.com/_draft_"),
            Some(ContentKind::Url)
        );
        assert_eq!(
            classify("C:\\users\\me\\notes.md"),
            Some(ContentKind::Path)
        );
        assert_eq!(classify("{\"title\":\"# heading\"}"), Some(ContentKind::Json));
    }
}
