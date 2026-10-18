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
    let digits = value.strip_prefix('#')?;
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
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
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.len() != 3 && parts.len() != 4 {
        return None;
    }

    let mut channels = [0u8; 3];
    for (slot, part) in channels.iter_mut().zip(&parts[..3]) {
        *slot = parse_channel(part)?;
    }
    if parts.len() == 4 {
        parse_alpha(parts[3])?;
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
        assert_eq!(parse("ff0000"), None, "the hash is required");
        assert_eq!(parse("#gggggg"), None);
    }
}
