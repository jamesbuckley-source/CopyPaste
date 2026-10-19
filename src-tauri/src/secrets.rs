//! High-precision detection of credentials in copied text.
//!
//! Only the confidence level is kept; a match never leaves this module, so the
//! app can warn about a clip without keeping another copy of the secret and
//! without writing any part of it to the log.

/// How sure the rules are that a payload holds a credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    Low,
    Medium,
    High,
}

impl Confidence {
    pub const fn as_str(self) -> &'static str {
        match self {
            Confidence::Low => "low",
            Confidence::Medium => "medium",
            Confidence::High => "high",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "low" => Some(Confidence::Low),
            "medium" => Some(Confidence::Medium),
            "high" => Some(Confidence::High),
            _ => None,
        }
    }
}

/// Prefixes that only ever appear on real credentials. Each entry is the prefix
/// and the minimum number of characters that must follow it.
const TOKEN_PREFIXES: &[(&str, usize)] = &[
    ("ghp_", 20),
    ("gho_", 20),
    ("ghu_", 20),
    ("ghs_", 20),
    ("ghr_", 20),
    ("github_pat_", 20),
    ("glpat-", 16),
    ("sk_live_", 16),
    ("rk_live_", 16),
    ("sk-proj-", 16),
    ("sk-ant-", 16),
    ("xoxb-", 10),
    ("xoxp-", 10),
    ("xoxa-", 10),
    ("xoxr-", 10),
    ("xoxs-", 10),
    ("AIza", 30),
    ("npm_", 30),
    ("SG.", 16),
    ("AKIA", 12),
    ("ASIA", 12),
    ("ya29.", 20),
    ("shpat_", 20),
    ("dop_v1_", 40),
    ("pypi-", 40),
];

/// Words that make a nearby random-looking string worth flagging (opt-in rule).
const CONTEXT_WORDS: &[&str] = &[
    "password",
    "passwd",
    "api_key",
    "apikey",
    "api-key",
    "access_key",
    "secret",
    "client_secret",
    "private_key",
    "token",
];

/// Scans `text` and reports how sure the rules are that it holds a credential.
///
/// `heuristics` enables the opt-in context and entropy rule, which trades a few
/// false positives for catching credentials of unknown shape.
pub fn detect(text: &str, heuristics: bool) -> Option<Confidence> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if has_pem_block(trimmed) || has_auth_header(trimmed) || has_credential_url(trimmed) {
        return Some(Confidence::High);
    }
    if has_known_token(trimmed) {
        return Some(Confidence::High);
    }
    if has_jwt(trimmed) {
        return Some(Confidence::Medium);
    }
    if heuristics && looks_like_secret_assignment(trimmed) {
        return Some(Confidence::Low);
    }
    None
}

fn has_pem_block(text: &str) -> bool {
    text.contains("-----BEGIN") && text.contains("PRIVATE KEY")
}

fn has_auth_header(text: &str) -> bool {
    text.lines().any(|line| {
        let lower = line.trim().to_ascii_lowercase();
        let Some(rest) = lower.strip_prefix("authorization:") else {
            return false;
        };
        let rest = rest.trim_start();
        (rest.starts_with("bearer ")
            || rest.starts_with("basic ")
            || rest.starts_with("token ")
            || rest.starts_with("digest "))
            && rest.len() > 12
    })
}

/// `scheme://user:secret@host` — a password pasted inside a URL.
fn has_credential_url(text: &str) -> bool {
    let Some(scheme_end) = text.find("://") else {
        return false;
    };
    let rest = &text[scheme_end + 3..];
    let authority = rest.split(['/', '?', '#', ' ', '\n']).next().unwrap_or("");
    let Some(at) = authority.find('@') else {
        return false;
    };
    let userinfo = &authority[..at];
    match userinfo.split_once(':') {
        Some((user, password)) => !user.is_empty() && password.len() >= 3,
        None => false,
    }
}

fn has_known_token(text: &str) -> bool {
    TOKEN_PREFIXES.iter().any(|(prefix, minimum)| {
        text.match_indices(prefix).any(|(index, _)| {
            let tail = &text[index + prefix.len()..];
            let run: String = tail
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            run.len() >= *minimum
        })
    })
}

/// Three base64url segments; the first starts with `eyJ` (`{"`).
fn has_jwt(text: &str) -> bool {
    let is_segment = |part: &str, minimum: usize| {
        part.len() >= minimum
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    };
    text.split(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',')
        .filter(|token| token.starts_with("eyJ"))
        .any(|token| {
            let mut parts = token.split('.');
            match (parts.next(), parts.next(), parts.next(), parts.next()) {
                (Some(header), Some(payload), Some(signature), None) => {
                    is_segment(header, 10) && is_segment(payload, 10) && is_segment(signature, 10)
                }
                _ => false,
            }
        })
}

/// `password = "…"` style assignments, or a keyword next to a random-looking run.
fn looks_like_secret_assignment(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    CONTEXT_WORDS.iter().any(|word| {
        lower.match_indices(word).any(|(index, _)| {
            let after = &text[index + word.len()..];
            let after = after.trim_start_matches(['"', '\'', ' ', '\t']);
            let after = after.strip_prefix(['=', ':']).unwrap_or(after).trim_start();
            let value: String = after
                .trim_start_matches(['"', '\''])
                .chars()
                .take_while(|c| c.is_ascii_graphic() && !"\"',;".contains(*c))
                .collect();
            value.len() >= 12 && entropy(&value) >= 3.3
        })
    })
}

/// Shannon entropy in bits per character over the distinct symbols in `value`.
fn entropy(value: &str) -> f64 {
    if value.is_empty() {
        return 0.0;
    }
    let mut counts = std::collections::HashMap::new();
    for byte in value.bytes() {
        *counts.entry(byte).or_insert(0usize) += 1;
    }
    let length = value.len() as f64;
    counts.values().fold(0.0, |sum, count| {
        let p = *count as f64 / length;
        sum - p * p.log2()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pem_private_key_is_high_confidence() {
        let text = "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----";
        assert_eq!(detect(text, false), Some(Confidence::High));
    }

    #[test]
    fn a_bearer_header_is_high_confidence() {
        assert_eq!(
            detect("Authorization: Bearer abcdef123456", false),
            Some(Confidence::High)
        );
    }

    #[test]
    fn a_url_with_a_password_is_high_confidence() {
        assert_eq!(
            detect("https://user:hunter2@example.com/path", false),
            Some(Confidence::High)
        );
        assert_eq!(detect("https://example.com/a?b=1", false), None);
    }

    #[test]
    fn known_token_prefixes_are_high_confidence() {
        assert_eq!(
            detect("ghp_abcdefghijklmnopqrstuvwxyz0123456789", false),
            Some(Confidence::High)
        );
        assert_eq!(
            detect("AKIAIOSFODNN7EXAMPLE", false),
            Some(Confidence::High)
        );
        // A bare mention of a prefix is not a credential.
        assert_eq!(detect("use a ghp_ token here", false), None);
    }

    #[test]
    fn a_jwt_is_medium_confidence() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(detect(jwt, false), Some(Confidence::Medium));
    }

    #[test]
    fn the_heuristic_only_runs_when_asked() {
        let text = "api_key = \"9f8a7b6c5d4e3f2a1b0c\"";
        assert_eq!(detect(text, false), None);
        assert_eq!(detect(text, true), Some(Confidence::Low));
    }

    #[test]
    fn ordinary_text_is_not_flagged() {
        assert_eq!(detect("Let's meet at 10 tomorrow.", true), None);
        assert_eq!(detect("", true), None);
        assert_eq!(detect("C:\\Windows\\System32", true), None);
    }

    #[test]
    fn confidence_round_trips_through_its_storage_form() {
        for value in [Confidence::Low, Confidence::Medium, Confidence::High] {
            assert_eq!(Confidence::parse(value.as_str()), Some(value));
        }
        assert_eq!(Confidence::parse("unknown"), None);
    }
}
