//! Relevance scoring for clipboard search.
//!
//! Clip bodies are stored locally, and a compact in-memory index mirrors the
//! searchable text so typing never has to scan the database. The index keeps
//! search-as-you-type synchronous and needs no extra native dependencies.
//!
//! Matching is substring based and ANDs the whitespace-separated terms, which is
//! what a user expects when they type a few fragments of something they copied.

/// Upper bound on how much of a single clip participates in matching. Keeps the
/// index bounded when someone copies a multi-megabyte document.
pub const MAX_INDEXED_CHARS: usize = 16 * 1024;

/// Lower-case and collapse runs of whitespace so that matching is insensitive to
/// how a string happened to be indented or wrapped.
pub fn normalize(input: &str) -> String {
    let mut out = String::new();
    let mut pending_space = false;
    let mut count = 0usize;

    for ch in input.chars() {
        if count >= MAX_INDEXED_CHARS {
            break;
        }
        if ch.is_whitespace() {
            // Defer emitting the space so trailing whitespace never lands in `out`.
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            count += 1;
            pending_space = false;
        }
        for lower in ch.to_lowercase() {
            if count >= MAX_INDEXED_CHARS {
                break;
            }
            out.push(lower);
            count += 1;
        }
    }

    out
}

#[derive(Debug, Clone)]
pub struct Query {
    raw: String,
    terms: Vec<String>,
}

impl Query {
    pub fn parse(input: &str) -> Self {
        let raw = normalize(input);
        // `str::split` yields one empty element for an empty string, which would
        // make a blank query look like a single empty term. Every non-empty text
        // then "matches" nothing useful, and clips with empty bodies drop out
        // entirely. Filter so a blank query really means "no filter".
        let terms = raw
            .split(' ')
            .filter(|term| !term.is_empty())
            .map(str::to_owned)
            .collect();
        Query { raw, terms }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }
}

/// Scores `text` against `query`, or `None` when it does not match at all.
///
/// `text` must already be [`normalize`]d. Higher is better.
pub fn score(query: &Query, text: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    if text.is_empty() {
        return None;
    }

    // Every term must appear somewhere (AND), and we track the left-most hit to
    // reward clips where the match shows up early.
    let mut earliest = usize::MAX;
    for term in &query.terms {
        let position = text.find(term.as_str())?;
        earliest = earliest.min(position);
    }

    let mut score = 100;
    if text == query.raw {
        score += 1200;
    } else if text.starts_with(query.raw.as_str()) {
        score += 600;
    } else if is_word_start(text, earliest) {
        score += 300;
    }

    // Prefer matches near the beginning, with the penalty saturating so that a
    // late match is merely "worse" rather than infinitely so.
    score -= (earliest.min(600) as i32) / 6;
    // Tie-break toward concise clips: a short paste is usually the intended hit.
    score -= (text.len().min(5_000) as i32) / 120;

    Some(score)
}

fn is_word_start(text: &str, index: usize) -> bool {
    if index == 0 {
        return true;
    }
    // Byte-wise check: a match preceded by a non-alphanumeric byte starts a word.
    // Non-ASCII predecessors read as non-alphanumeric, which is the safe default.
    text.as_bytes()
        .get(index - 1)
        .is_none_or(|b| !b.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rank(query: &str, candidates: &[&str]) -> Vec<String> {
        let q = Query::parse(query);
        let mut scored: Vec<(i32, String)> = candidates
            .iter()
            .filter_map(|c| score(&q, &normalize(c)).map(|s| (s, (*c).to_string())))
            .collect();
        scored.sort_by_key(|a| std::cmp::Reverse(a.0));
        scored.into_iter().map(|(_, s)| s).collect()
    }

    #[test]
    fn normalize_lowercases_and_collapses_whitespace() {
        assert_eq!(
            normalize("  Hello   WORLD \n\t again "),
            "hello world again"
        );
        assert_eq!(normalize(""), "");
        assert_eq!(normalize("   "), "");
    }

    #[test]
    fn normalize_is_bounded() {
        let huge = "a".repeat(MAX_INDEXED_CHARS * 4);
        assert!(normalize(&huge).chars().count() <= MAX_INDEXED_CHARS);
    }

    #[test]
    fn empty_query_matches_everything_including_blank_bodies() {
        let q = Query::parse("   ");
        assert!(q.is_empty());
        assert_eq!(
            score(&q, ""),
            Some(0),
            "a blank query must not filter anything out"
        );
        assert_eq!(score(&q, "anything"), Some(0));
        assert!(Query::parse("").is_empty(), "empty input is an empty query");
    }

    #[test]
    fn all_terms_must_match() {
        assert!(score(&Query::parse("alpha beta"), "alpha only").is_none());
        assert!(score(&Query::parse("alpha beta"), "alpha and beta").is_some());
    }

    #[test]
    fn search_is_substring_and_case_insensitive() {
        assert!(score(&Query::parse("ELLO WOR"), "hello world").is_some());
        assert!(score(&Query::parse("world"), "hello world").is_some());
    }

    #[test]
    fn exact_match_outranks_prefix_which_outranks_substring() {
        let ranked = rank("config", &["config", "config file", "my config here"]);
        assert_eq!(ranked, vec!["config", "config file", "my config here"]);
    }

    #[test]
    fn earlier_matches_outrank_later_ones() {
        let ranked = rank(
            "needle",
            &[
                "needle in a haystack",
                &format!("{}needle", "x".repeat(400)),
            ],
        );
        assert_eq!(ranked[0], "needle in a haystack");
    }

    #[test]
    fn shorter_clips_win_ties() {
        let ranked = rank("token", &["token", &format!("token{}", "y".repeat(300))]);
        assert_eq!(ranked[0], "token");
    }

    #[test]
    fn word_boundary_matches_outrank_mid_word_matches() {
        let ranked = rank("log", &["log rotation", "catalog"]);
        assert_eq!(ranked[0], "log rotation");
    }

    #[test]
    fn non_ascii_content_does_not_panic() {
        assert!(score(&Query::parse("日本"), "日本語のテキスト").is_some());
        assert!(score(&Query::parse("café"), "crème café noir").is_some());
    }

    #[test]
    fn multi_term_scoring_uses_leftmost_hit() {
        // `beta` appears first; the recency-of-match penalty must use that, not
        // the position of the last term.
        let q = Query::parse("beta alpha");
        let early = score(&q, "beta alpha").unwrap();
        let late = score(&q, &format!("{}beta alpha", "z".repeat(500))).unwrap();
        assert!(early > late);
    }
}
