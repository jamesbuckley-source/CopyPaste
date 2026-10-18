//! Colour codes copied as text.
//!
//! A clip is treated as a colour only when its *entire* payload is a colour, so a
//! document that happens to mention `#ff0000` is not swallowed by the swatch view.

/// Normalises a whole clipboard payload to `#rrggbb`, or `None` when it is not a
/// single colour value.
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
        // A bare value is only a colour when it looks like one: hex digits that
        // include both a number and a letter. Without that test "facade" and
        // "decade" would be colours, and so would the number 123456.
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

    // CSS Colour 4 accepts spaces instead of commas and a `/` before the alpha, so
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
}

impl ContentKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            ContentKind::Color => "color",
            ContentKind::Url => "url",
            ContentKind::Email => "email",
            ContentKind::Path => "path",
            ContentKind::Json => "json",
        }
    }
}

/// Classifies a whole text payload, so the UI can offer matching actions.
pub fn classify(text: &str) -> Option<ContentKind> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.lines().count() != 1 {
        return None;
    }
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
    None
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
        assert_eq!(parse("the colour is #ff0000"), None, "not the whole payload");
        assert_eq!(parse("#ff0000 on a red background"), None);
        assert_eq!(parse("#ff0000\n#00ff00"), None, "multi-line payloads are text");
        assert_eq!(parse("#12345"), None);
        assert_eq!(parse("rgb(256, 0, 0)"), None);
        assert_eq!(parse("rgb(1,2,3,4,5)"), None);
        assert_eq!(parse("rgb(1,2,3, 2)"), None);
        assert_eq!(parse("#gggggg"), None);
        // Bare hex is only taken when it looks like a colour rather than a word.
        assert_eq!(parse("facade"), None, "six hex letters that spell a word");
        assert_eq!(parse("decade"), None);
        assert_eq!(parse("beaded"), None);
        assert_eq!(parse("123456"), None, "a bare number is not a colour");
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
    fn accepts_css_colour_4_rgb_syntax() {
        assert_eq!(parse("rgb(255 136 0)").as_deref(), Some("#ff8800"));
        assert_eq!(parse("rgb(255 136 0 / 0.5)").as_deref(), Some("#ff8800"));
        assert_eq!(parse("rgb(255 136 0 / 50%)").as_deref(), Some("#ff8800"));
        assert_eq!(parse("rgba(255 136 0 / 1)").as_deref(), Some("#ff8800"));
        assert_eq!(parse("rgb(255, 136, 0 / 50%)").as_deref(), Some("#ff8800"));
        assert_eq!(parse("rgb(255 136)"), None, "two channels are not a colour");
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
}
