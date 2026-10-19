//! Recovers the full animation behind a browser's "Copy image" of a GIF.
//!
//! Windows has no animated-GIF clipboard format: a browser rasterises the frame
//! it is showing into `CF_DIB`/PNG, so [`crate::clipboard`] only ever sees a
//! still image. The URL survives in the `CF_HTML` fragment the browser also
//! publishes, so capture can fetch the source asset and keep it beside the
//! frame. Copying then republishes that file through `CF_HDROP`, which every
//! animation-aware target understands.
//!
//! This is deliberately not part of [`crate::giphy`]: that module is the GIF
//! utility's allowlisted client, while this fetcher has to accept arbitrary
//! hosts and must not touch the GIPHY key. Relaxing the host rule opens an SSRF
//! surface, so the request path is HTTPS-only, refuses credentials and non-public
//! addresses, pins redirects to the same host, and validates the returned bytes.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
use std::time::Duration;

use base64::Engine as _;

use crate::error::{AppError, Result};

/// Kept in step with the GIF utility's cap so a hostile page cannot stream an
/// unbounded body into memory.
pub const MAX_GIF_BYTES: usize = 32 * 1024 * 1024;
/// Inline `data:` images have to be decoded in the capture process, so they get a
/// much smaller ceiling.
const MAX_INLINE_BYTES: usize = 2 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_REDIRECTS: usize = 3;

/// Where a candidate animation came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GifSource {
    /// An absolute `https://` address taken from an `<img>` attribute.
    Remote(String),
    /// A small `data:image/gif;base64,` payload already in the fragment.
    Inline(Vec<u8>),
}

/// Finds the first GIF-looking image in a `CF_HTML` fragment.
///
/// `src` wins over `srcset`, and `srcset` entries are tried in order. Anything
/// that is not clearly a GIF is skipped, so a copied JPEG or a page logo does not
/// trigger a download.
pub fn find(html: &str) -> Option<GifSource> {
    for tag in img_tags(html) {
        let attributes = attributes(tag);
        let mut candidates: Vec<&str> = Vec::new();
        if let Some(src) = attributes.get("src") {
            candidates.push(src);
        }
        if let Some(srcset) = attributes.get("srcset") {
            candidates.extend(
                srcset
                    .split(',')
                    .filter_map(|entry| entry.split_whitespace().next()),
            );
        }
        for candidate in candidates {
            if let Some(source) = classify(candidate) {
                return Some(source);
            }
        }
    }
    None
}

/// Fetches or decodes one candidate, returning bytes already checked to be a GIF.
pub async fn load(source: &GifSource) -> Result<Vec<u8>> {
    match source {
        GifSource::Inline(bytes) => {
            if !is_gif(bytes) {
                return Err(AppError::msg("the inline image was not a GIF"));
            }
            Ok(bytes.clone())
        }
        GifSource::Remote(url) => fetch(url).await,
    }
}

pub fn is_gif(bytes: &[u8]) -> bool {
    bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")
}

fn classify(raw: &str) -> Option<GifSource> {
    let candidate = raw
        .trim()
        .trim_matches('"')
        .replace("&amp;", "&")
        .replace("&#38;", "&");
    if candidate.is_empty() {
        return None;
    }
    if candidate.len() >= MAX_INLINE_BYTES {
        return None;
    }
    if let Some(rest) = candidate.strip_prefix("data:") {
        return decode_inline(rest);
    }
    // Page-scoped URLs cannot be fetched from another process.
    if candidate.starts_with("blob:") {
        return None;
    }
    let url = reqwest::Url::parse(&candidate).ok()?;
    if url.scheme() != "https" {
        return None;
    }
    if !url.path().to_ascii_lowercase().ends_with(".gif") {
        return None;
    }
    Some(GifSource::Remote(url.to_string()))
}

fn decode_inline(rest: &str) -> Option<GifSource> {
    let (meta, payload) = rest.split_once(',')?;
    let meta = meta.to_ascii_lowercase();
    if meta != "image/gif;base64" {
        return None;
    }
    let encoded = payload.trim();
    if encoded.len() > MAX_INLINE_BYTES {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded.as_bytes())
        .ok()?;
    if bytes.len() > MAX_INLINE_BYTES || !is_gif(&bytes) {
        return None;
    }
    Some(GifSource::Inline(bytes))
}

/// Splits the fragment into `<img ...>` tags, uppercasing nothing so attribute
/// values keep their case.
fn img_tags(html: &str) -> Vec<&str> {
    let lower = html.to_ascii_lowercase();
    let mut tags = Vec::new();
    let mut cursor = 0;
    while let Some(found) = lower[cursor..].find("<img") {
        let start = cursor + found;
        let Some(end) = lower[start..].find('>') else {
            break;
        };
        tags.push(&html[start..start + end]);
        cursor = start + end;
    }
    tags
}

/// Minimal attribute reader: enough for the quoted and unquoted forms browsers
/// emit, without pulling in an HTML parser for one job.
fn attributes(tag: &str) -> HashMap<String, String> {
    let bytes = tag.as_bytes();
    let mut index = 4; // past "<img"
    let mut attributes = HashMap::new();

    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let name_start = index;
        while index < bytes.len()
            && !bytes[index].is_ascii_whitespace()
            && bytes[index] != b'='
            && bytes[index] != b'>'
        {
            index += 1;
        }
        if index == name_start {
            break;
        }
        let name = tag[name_start..index].to_ascii_lowercase();
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'=' {
            continue;
        }
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        let value = if bytes[index] == b'"' || bytes[index] == b'\'' {
            let quote = bytes[index];
            index += 1;
            let value_start = index;
            while index < bytes.len() && bytes[index] != quote {
                index += 1;
            }
            let value = &tag[value_start..index];
            index = (index + 1).min(bytes.len());
            value
        } else {
            let value_start = index;
            while index < bytes.len() && !bytes[index].is_ascii_whitespace() && bytes[index] != b'>' {
                index += 1;
            }
            &tag[value_start..index]
        };
        attributes.entry(name).or_insert_with(|| value.to_string());
    }
    attributes
}

async fn fetch(url: &str) -> Result<Vec<u8>> {
    let initial = validate_remote(url)?;
    let host = initial
        .host_str()
        .ok_or_else(|| AppError::msg("that GIF address has no host"))?
        .to_ascii_lowercase();

    let client = reqwest::Client::builder()
        .user_agent("CopyPaste (Windows clipboard manager)")
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.error("too many redirects");
            }
            let target = attempt.url();
            if target.scheme() != "https" || target.port_or_known_default() != Some(443) {
                return attempt.error("a redirect left HTTPS");
            }
            match target.host_str() {
                Some(next) if next.eq_ignore_ascii_case(&host) => attempt.follow(),
                _ => attempt.error("cross-host redirect refused"),
            }
        }))
        .build()
        .map_err(|e| AppError::msg(format!("could not prepare the GIF request: {e}")))?;

    let response = client
        .get(initial)
        .header(reqwest::header::ACCEPT, "image/gif,image/*;q=0.8")
        .send()
        .await
        .map_err(|e| AppError::msg(format!("could not download the GIF: {e}")))?;
    let status = response.status();
    if !status.is_success() {
        return Err(AppError::msg(format!("the image host returned HTTP {status}")));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !content_type.starts_with("image/gif") {
        return Err(AppError::msg("the image host did not return a GIF"));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_GIF_BYTES as u64)
    {
        return Err(AppError::msg("that GIF is too large"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| AppError::msg(format!("could not read the GIF: {e}")))?;
    if bytes.len() > MAX_GIF_BYTES {
        return Err(AppError::msg("that GIF is too large"));
    }
    if !is_gif(&bytes) {
        return Err(AppError::msg("the image host did not return a GIF"));
    }
    Ok(bytes.to_vec())
}

/// Checks the address before any request is made. Hostnames are resolved here so
/// a public-looking name that points at the loopback or a private range is
/// rejected too.
fn validate_remote(url: &str) -> Result<reqwest::Url> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| AppError::msg("that is not a valid image address"))?;
    if parsed.scheme() != "https" {
        return Err(AppError::msg("image addresses must use HTTPS"));
    }
    if !matches!(parsed.port(), None | Some(443)) {
        return Err(AppError::msg("image addresses must use the standard HTTPS port"));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(AppError::msg("image addresses cannot carry credentials"));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| AppError::msg("that image address has no host"))?;
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") {
        return Err(AppError::msg("only public image hosts can be fetched"));
    }
    let addresses: Vec<IpAddr> = (host, 443u16)
        .to_socket_addrs()
        .map_err(|_| AppError::msg("that image host could not be resolved"))?
        .map(|address| address.ip())
        .collect();
    if addresses.is_empty() {
        return Err(AppError::msg("that image host could not be resolved"));
    }
    if addresses.iter().any(|address| !is_public(address)) {
        return Err(AppError::msg("only public image hosts can be fetched"));
    }
    Ok(parsed)
}

fn is_public(address: &IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => is_public_v4(*v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_v4(v4);
            }
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00 // unique local
                || (first & 0xffc0) == 0xfe80) // link local
        }
    }
}

fn is_public_v4(v4: Ipv4Addr) -> bool {
    let octets = v4.octets();
    !(v4.is_private()
        || v4.is_loopback()
        || v4.is_link_local()
        || v4.is_broadcast()
        || v4.is_documentation()
        || v4.is_unspecified()
        || v4.is_multicast()
        // CGNAT 100.64.0.0/10.
        || (octets[0] == 100 && (64..=127).contains(&octets[1])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_gif_in_an_unquoted_src() {
        let html = r#"<html><body><!--StartFragment--><img src=https://media.example.com/a.gif width=10><!--EndFragment--></body></html>"#;
        assert_eq!(
            find(html),
            Some(GifSource::Remote("https://media.example.com/a.gif".into()))
        );
    }

    #[test]
    fn prefers_srcset_entries_over_non_gif_sources() {
        let html = r#"<img src="https://example.com/preview.png" srcset="https://example.com/a.png 1x, https://example.com/a.gif 2x" alt="">"#;
        assert_eq!(
            find(html),
            Some(GifSource::Remote("https://example.com/a.gif".into()))
        );
    }

    #[test]
    fn ignores_non_gif_sources_and_blob_urls() {
        assert_eq!(find(r#"<img src="https://example.com/a.png">"#), None);
        assert_eq!(find(r#"<img src="blob:https://example.com/1234">"#), None);
        assert_eq!(find("<p>no images here</p>"), None);
    }

    #[test]
    fn keeps_query_strings_on_gif_urls() {
        let html = r#"<img src="https://example.com/a.gif?width=10&amp;height=20">"#;
        assert_eq!(
            find(html),
            Some(GifSource::Remote(
                "https://example.com/a.gif?width=10&height=20".into()
            ))
        );
    }

    #[test]
    fn decodes_a_bounded_inline_gif() {
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"GIF89a demo");
        let html = format!(r#"<img src="data:image/gif;base64,{encoded}">"#);
        assert_eq!(
            find(&html),
            Some(GifSource::Inline(b"GIF89a demo".to_vec()))
        );
    }

    #[test]
    fn rejects_private_and_loopback_addresses() {
        assert!(is_public(&"127.0.0.1".parse().unwrap()) == false);
        assert!(is_public(&"10.1.2.3".parse().unwrap()) == false);
        assert!(is_public(&"192.168.0.1".parse().unwrap()) == false);
        assert!(is_public(&"169.254.1.1".parse().unwrap()) == false);
        assert!(is_public(&"100.64.0.1".parse().unwrap()) == false);
        assert!(is_public(&"::1".parse().unwrap()) == false);
        assert!(is_public(&"fc00::1".parse().unwrap()) == false);
        assert!(is_public(&"::ffff:127.0.0.1".parse().unwrap()) == false);
        assert!(is_public(&"93.184.216.34".parse().unwrap()));
    }

    #[test]
    fn rejects_plain_http_and_credentialed_addresses() {
        assert!(validate_remote("http://example.com/a.gif").is_err());
        assert!(validate_remote("https://user:pass@example.com/a.gif").is_err());
        assert!(validate_remote("https://example.com:8443/a.gif").is_err());
        assert!(validate_remote("https://localhost/a.gif").is_err());
    }

    #[test]
    fn sniffs_gif_headers() {
        assert!(is_gif(b"GIF87a rest"));
        assert!(is_gif(b"GIF89a rest"));
        assert!(!is_gif(b"\x89PNG\r\n\x1a\n"));
    }
}
