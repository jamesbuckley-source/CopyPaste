//! Recovers the moving asset behind a browser's "Copy image" of a GIF or video.
//!
//! Windows has no animated-GIF or video clipboard format: a browser rasterises
//! the frame it is showing into `CF_DIB`/PNG, so [`crate::clipboard`] only ever
//! sees a still image. The URL survives in the `CF_HTML` fragment the browser
//! also publishes, so capture can fetch the source asset and keep it beside the
//! frame. Copying then republishes that file through `CF_HDROP`, which every
//! animation-aware target understands.
//!
//! An MP4 is kept as an MP4. Pages such as Reddit serve one under a `.gif` path
//! with `?format=mp4`, and transcoding or renaming it would either cost a video
//! codec in the build or hand a player bytes its name lies about.
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
/// Videos are capped for exactly the same reason. A short clip from a page fits
/// well inside this; a feature-length file does not belong in a clipboard history.
pub const MAX_VIDEO_BYTES: usize = 32 * 1024 * 1024;
/// Inline `data:` images have to be decoded in the capture process, so they get a
/// much smaller ceiling.
const MAX_INLINE_BYTES: usize = 2 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_REDIRECTS: usize = 3;

/// Which moving asset a source holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionKind {
    Gif,
    Video,
}

impl MotionKind {
    /// Extension the asset is stored under beside the clip's still frame.
    pub const fn extension(self) -> &'static str {
        match self {
            MotionKind::Gif => "gif",
            MotionKind::Video => "mp4",
        }
    }

    pub const fn is_video(self) -> bool {
        matches!(self, MotionKind::Video)
    }
}

/// Where a candidate moving asset came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MotionSource {
    /// An absolute `https://` address taken from a media attribute.
    Remote { kind: MotionKind, url: String },
    /// A small `data:image/gif;base64,` payload already in the fragment.
    Inline(Vec<u8>),
}

impl MotionSource {
    pub const fn kind(&self) -> MotionKind {
        match self {
            MotionSource::Remote { kind, .. } => *kind,
            MotionSource::Inline(_) => MotionKind::Gif,
        }
    }
}

/// Finds the first GIF or video in a `CF_HTML` fragment.
///
/// `src` wins over `srcset` and `poster`, and `srcset` entries are tried in
/// order. Anything that is not clearly a GIF or an MP4 is skipped, so a copied
/// JPEG or a page logo does not trigger a download.
pub fn find(html: &str) -> Option<MotionSource> {
    for tag in motion_tags(html) {
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
        // Reddit keeps the clip in a `<video>` element's poster or in a nested
        // `<source>`, so those attributes are worth a look after the direct ones.
        if let Some(poster) = attributes.get("poster") {
            candidates.push(poster);
        }
        for candidate in candidates {
            if let Some(source) = classify(candidate) {
                return Some(source);
            }
        }
    }
    None
}

/// Fetches or decodes one candidate, returning bytes already checked to match
/// the kind the source promised.
pub async fn load(source: &MotionSource) -> Result<Vec<u8>> {
    match source {
        MotionSource::Inline(bytes) => {
            if !is_gif(bytes) {
                return Err(AppError::msg("the inline image was not a GIF"));
            }
            Ok(bytes.clone())
        }
        MotionSource::Remote { kind, url } => fetch(url, *kind).await,
    }
}

pub fn is_gif(bytes: &[u8]) -> bool {
    bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")
}

/// An ISO base media file starts with a box whose type is `ftyp`.
///
/// That covers MP4, M4V and QuickTime alike, which is exactly what a page's
/// "video" link tends to serve.
pub fn is_mp4(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && &bytes[4..8] == b"ftyp"
}

fn noun(kind: MotionKind) -> &'static str {
    match kind {
        MotionKind::Gif => "GIF",
        MotionKind::Video => "video",
    }
}

fn max_bytes(kind: MotionKind) -> usize {
    match kind {
        MotionKind::Gif => MAX_GIF_BYTES,
        MotionKind::Video => MAX_VIDEO_BYTES,
    }
}

fn accepts_content_type(kind: MotionKind, content_type: &str) -> bool {
    match kind {
        MotionKind::Gif => content_type.starts_with("image/gif"),
        MotionKind::Video => {
            content_type.starts_with("video/mp4") || content_type.starts_with("application/mp4")
        }
    }
}

fn looks_like(kind: MotionKind, bytes: &[u8]) -> bool {
    match kind {
        MotionKind::Gif => is_gif(bytes),
        MotionKind::Video => is_mp4(bytes),
    }
}

/// Rejects a body that is the wrong kind or over the cap, before it is stored.
fn check_payload(kind: MotionKind, bytes: &[u8]) -> Result<()> {
    if bytes.len() > max_bytes(kind) {
        return Err(AppError::msg(format!("that {} is too large", noun(kind))));
    }
    if !looks_like(kind, bytes) {
        return Err(AppError::msg(format!(
            "the host did not return a {}",
            noun(kind)
        )));
    }
    Ok(())
}

/// Recognises a clipboard payload that is nothing but a link to a GIF or video.
///
/// "Copy video address" is a text copy, so pasting one would otherwise be just a
/// link. This is what lets that paste take the same path as a browser's "Copy
/// image": the asset is fetched and kept beside the clip. The whole payload has
/// to be the address — a sentence that mentions one stays text.
pub fn from_url(text: &str) -> Option<MotionSource> {
    let candidate = text.trim();
    if candidate.is_empty() || candidate.chars().any(char::is_whitespace) {
        return None;
    }
    classify(candidate)
}

fn classify(raw: &str) -> Option<MotionSource> {
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
    let kind = kind_of(&url)?;
    Some(MotionSource::Remote {
        kind,
        url: url.to_string(),
    })
}

/// Which asset an address points at.
///
/// The query is checked before the path because Reddit and similar pages put an
/// MP4 under a `.gif` name and say so only in `?format=mp4`.
fn kind_of(url: &reqwest::Url) -> Option<MotionKind> {
    let wants_mp4 = url.query_pairs().any(|(key, value)| {
        key.eq_ignore_ascii_case("format") && value.eq_ignore_ascii_case("mp4")
    });
    if wants_mp4 {
        return Some(MotionKind::Video);
    }
    let path = url.path().to_ascii_lowercase();
    if path.ends_with(".mp4") || path.ends_with(".m4v") {
        return Some(MotionKind::Video);
    }
    if path.ends_with(".gif") {
        return Some(MotionKind::Gif);
    }
    None
}

fn decode_inline(rest: &str) -> Option<MotionSource> {
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
    Some(MotionSource::Inline(bytes))
}

/// Every `<img>`, `<video>` and `<source>` tag in the fragment, in the order they
/// appear, uppercasing nothing so attribute values keep their case.
///
/// `<source>` matters because a `<video>` element usually carries its address
/// there rather than on the element itself.
fn motion_tags(html: &str) -> Vec<&str> {
    let lower = html.to_ascii_lowercase();
    let mut found: Vec<(usize, usize)> = Vec::new();
    for name in ["<img", "<video", "<source"] {
        let mut cursor = 0;
        while let Some(offset) = lower[cursor..].find(name) {
            let start = cursor + offset;
            let Some(end) = lower[start..].find('>') else {
                break;
            };
            found.push((start, start + end));
            cursor = start + end;
        }
    }
    found.sort_unstable();
    found
        .into_iter()
        .map(|(start, end)| &html[start..end])
        .collect()
}

/// Minimal attribute reader: enough for the quoted and unquoted forms browsers
/// emit, without pulling in an HTML parser for one job.
fn attributes(tag: &str) -> HashMap<String, String> {
    let bytes = tag.as_bytes();
    // Past the tag name, whichever element this is (`<img`, `<video`, `<source`).
    let mut index = tag.find(char::is_whitespace).unwrap_or(tag.len());
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

async fn fetch(url: &str, kind: MotionKind) -> Result<Vec<u8>> {
    let initial = validate_remote(url)?;
    let host = initial
        .host_str()
        .ok_or_else(|| AppError::msg("that media address has no host"))?
        .to_ascii_lowercase();
    let label = noun(kind);

    let client = guarded_client(&host)
        .map_err(|e| AppError::msg(format!("could not prepare the {label} request: {e}")))?;

    let accept = match kind {
        MotionKind::Gif => "image/gif,image/*;q=0.8",
        MotionKind::Video => "video/mp4,video/*;q=0.9,*/*;q=0.5",
    };
    let response = client
        .get(initial)
        .header(reqwest::header::ACCEPT, accept)
        .send()
        .await
        .map_err(|e| AppError::msg(format!("could not download the {label}: {e}")))?;
    let status = response.status();
    if !status.is_success() {
        return Err(AppError::msg(format!(
            "the media host returned HTTP {status}"
        )));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !accepts_content_type(kind, &content_type) {
        return Err(AppError::msg(format!(
            "the media host did not return a {label}"
        )));
    }
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes(kind) as u64)
    {
        return Err(AppError::msg(format!("that {label} is too large")));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| AppError::msg(format!("could not read the {label}: {e}")))?;
    check_payload(kind, &bytes)?;
    Ok(bytes.to_vec())
}

/// A client that will only talk HTTPS to `host`, and only follow redirects that
/// stay on it.
///
/// Shared with [`crate::preview`]: both fetch addresses a page handed us, and the
/// redirect rule is the part that stops a public host bouncing the request into
/// the local network.
pub(crate) fn guarded_client(host: &str) -> Result<reqwest::Client> {
    let host = host.to_ascii_lowercase();
    reqwest::Client::builder()
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
        .map_err(|e| AppError::msg(format!("could not prepare the request: {e}")))
}

/// Checks the address before any request is made. Hostnames are resolved here so
/// a public-looking name that points at the loopback or a private range is
/// rejected too.
pub(crate) fn validate_remote(url: &str) -> Result<reqwest::Url> {
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

    fn remote(kind: MotionKind, url: &str) -> Option<MotionSource> {
        Some(MotionSource::Remote {
            kind,
            url: url.into(),
        })
    }

    #[test]
    fn finds_a_gif_in_an_unquoted_src() {
        let html = r#"<html><body><!--StartFragment--><img src=https://media.example.com/a.gif width=10><!--EndFragment--></body></html>"#;
        assert_eq!(
            find(html),
            remote(MotionKind::Gif, "https://media.example.com/a.gif")
        );
    }

    #[test]
    fn prefers_srcset_entries_over_non_gif_sources() {
        let html = r#"<img src="https://example.com/preview.png" srcset="https://example.com/a.png 1x, https://example.com/a.gif 2x" alt="">"#;
        assert_eq!(
            find(html),
            remote(MotionKind::Gif, "https://example.com/a.gif")
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
            remote(MotionKind::Gif, "https://example.com/a.gif?width=10&height=20")
        );
    }

    #[test]
    fn decodes_a_bounded_inline_gif() {
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"GIF89a demo");
        let html = format!(r#"<img src="data:image/gif;base64,{encoded}">"#);
        assert_eq!(
            find(&html),
            Some(MotionSource::Inline(b"GIF89a demo".to_vec()))
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
    fn sniffs_gif_and_mp4_headers() {
        assert!(is_gif(b"GIF87a rest"));
        assert!(is_gif(b"GIF89a rest"));
        assert!(!is_gif(b"\x89PNG\r\n\x1a\n"));

        assert!(is_mp4(b"\0\0\0\x18ftypisomrest"));
        assert!(!is_mp4(b"GIF89a demo"));
        assert!(!is_mp4(b"ftyp"));
    }

    #[test]
    fn a_gif_path_that_asks_for_mp4_is_a_video() {
        // The shape Reddit publishes: `.gif` in the path, MP4 in the query.
        let html = r#"<img src="https://preview.redd.it/abc.gif?width=640&amp;format=mp4&amp;s=deadbeef" alt="">"#;
        assert_eq!(
            find(html),
            remote(
                MotionKind::Video,
                "https://preview.redd.it/abc.gif?width=640&format=mp4&s=deadbeef"
            )
        );
    }

    #[test]
    fn finds_a_video_element_or_its_nested_source() {
        let direct = r#"<video src="https://cdn.example.com/clip.mp4" controls></video>"#;
        assert_eq!(
            find(direct),
            remote(MotionKind::Video, "https://cdn.example.com/clip.mp4")
        );

        let nested = r#"<video poster="https://cdn.example.com/clip.jpg"><source src="https://cdn.example.com/clip.mp4" type="video/mp4"></video>"#;
        assert_eq!(
            find(nested),
            remote(MotionKind::Video, "https://cdn.example.com/clip.mp4")
        );
    }

    #[test]
    fn a_video_element_without_src_falls_back_to_its_poster() {
        let html = r#"<video poster="https://cdn.example.com/still.gif"></video>"#;
        assert_eq!(
            find(html),
            remote(MotionKind::Gif, "https://cdn.example.com/still.gif")
        );
    }

    #[test]
    fn a_pasted_movie_address_is_recognised() {
        let pasted = "https://external-preview.redd.it/WkPLmQSB_QyBc3X3UCnCXPfWkKGXCTCeCF59mpDBQHY.gif?width=268&format=mp4&s=067b3e18daf1dd7ff71018a2fe6a7d906a8371ec";
        assert_eq!(from_url(pasted), remote(MotionKind::Video, pasted));

        assert_eq!(
            from_url("  https://example.com/a.gif\n"),
            remote(MotionKind::Gif, "https://example.com/a.gif")
        );
        assert_eq!(
            from_url("https://example.com/a.mp4"),
            remote(MotionKind::Video, "https://example.com/a.mp4")
        );
    }

    #[test]
    fn a_sentence_that_mentions_a_movie_address_stays_text() {
        assert_eq!(from_url("see https://example.com/a.mp4"), None);
        assert_eq!(from_url("https://example.com/a.mp4 and more"), None);
        assert_eq!(from_url(""), None);
        assert_eq!(from_url("https://example.com/page.html"), None);
        assert_eq!(from_url("http://example.com/a.mp4"), None);
        assert_eq!(from_url("blob:https://example.com/a.mp4"), None);
    }

    #[test]
    fn content_types_are_matched_per_kind() {        assert!(accepts_content_type(MotionKind::Gif, "image/gif"));
        assert!(accepts_content_type(MotionKind::Gif, "image/gif; charset=binary"));
        assert!(!accepts_content_type(MotionKind::Gif, "video/mp4"));
        assert!(accepts_content_type(MotionKind::Video, "video/mp4"));
        assert!(accepts_content_type(MotionKind::Video, "application/mp4"));
        assert!(!accepts_content_type(MotionKind::Video, "image/gif"));
        assert!(!accepts_content_type(MotionKind::Video, "text/html"));
    }

    #[test]
    fn a_body_of_the_wrong_kind_is_rejected() {
        // An MP4 under a `.gif` name with no `format=mp4` still fails the sniff, so
        // the clip keeps its still frame rather than storing mislabelled bytes.
        assert!(check_payload(MotionKind::Gif, b"\0\0\0\x18ftypisom").is_err());
        assert!(check_payload(MotionKind::Video, b"GIF89a demo").is_err());
        assert!(check_payload(MotionKind::Video, b"\0\0\0\x18ftypisom").is_ok());
    }

    #[test]
    fn an_oversized_body_is_rejected() {
        let mut huge = vec![0u8; MAX_VIDEO_BYTES + 1];
        huge[4..8].copy_from_slice(b"ftyp");
        let error = check_payload(MotionKind::Video, &huge).unwrap_err().to_string();
        assert!(error.contains("too large"), "unexpected message: {error}");
    }
}
