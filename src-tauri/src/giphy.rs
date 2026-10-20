//! GIPHY search and GIF downloads for the GIF utility.
//!
//! Everything network-facing lives in the Rust process: the webview keeps a
//! `connect-src 'self'` policy, and the frontend only ever asks for a search or a
//! copy. Downloads are restricted to GIPHY hosts so a manipulated payload cannot
//! turn the command into an arbitrary HTTP client.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::error::{AppError, Result};
use crate::hashing;

/// GIPHY retired the public beta key that used to appear in their docs, so the
/// utility asks the user for their own (free) key instead of pretending to work.
const MISSING_KEY: &str =
    "add a GIPHY API key in Settings to search (a free key takes a minute at developers.giphy.com)";
const MAX_GIF_BYTES: usize = 32 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// One GIF as the browser needs it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GifResult {
    pub id: String,
    pub title: String,
    /// Original animation, used when copying.
    pub url: String,
    /// Smaller animation, used for the grid so browsing stays quick.
    pub preview_url: String,
    /// Static first frame, swapped in when previews only animate on hover.
    pub still_url: String,
    pub width: u32,
    pub height: u32,
}

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("CopyPaste (Windows clipboard manager)")
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| AppError::msg(format!("could not prepare the GIPHY request: {e}")))
}

/// Searches GIPHY, or asks for trending GIFs when the query is blank.
pub async fn search(configured_key: &str, query: &str, limit: usize) -> Result<Vec<GifResult>> {
    let key = effective_key(configured_key)?;
    let query = query.trim();
    let endpoint = if query.is_empty() {
        "https://api.giphy.com/v1/gifs/trending"
    } else {
        "https://api.giphy.com/v1/gifs/search"
    };
    let limit = limit.clamp(1, 48).to_string();

    let mut request = client()?
        .get(endpoint)
        .query(&[("api_key", key), ("limit", limit.as_str()), ("rating", "pg-13")]);
    if !query.is_empty() {
        request = request.query(&[("q", query)]);
    }

    let response = request
        .send()
        .await
        .map_err(|e| AppError::msg(format!("could not reach GIPHY: {e}")))?;
    let status = response.status();
    let payload: Value = response
        .json()
        .await
        .map_err(|e| AppError::msg(format!("GIPHY returned an unreadable response: {e}")))?;
    parse_payload(status.as_u16(), &payload)
}

fn effective_key(configured: &str) -> Result<&str> {
    let trimmed = configured.trim();
    if trimmed.is_empty() {
        return Err(AppError::msg(MISSING_KEY));
    }
    Ok(trimmed)
}

fn parse_payload(http_status: u16, payload: &Value) -> Result<Vec<GifResult>> {
    let meta_status = payload
        .pointer("/meta/status")
        .and_then(Value::as_u64)
        .unwrap_or(u64::from(http_status));
    if !(200..300).contains(&http_status) || meta_status != 200 {
        let message = payload
            .pointer("/meta/msg")
            .and_then(Value::as_str)
            .filter(|message| !message.trim().is_empty())
            .unwrap_or("the request was rejected");
        return Err(AppError::msg(format!("GIPHY said: {message}")));
    }

    let items = payload
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::msg("GIPHY returned no data"))?;
    let mut results = Vec::with_capacity(items.len());
    for item in items {
        let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
        let images = item.get("images").unwrap_or(&Value::Null);
        let Some(url) = images
            .pointer("/original/url")
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
        else {
            continue;
        };
        let preview_url = images
            .pointer("/fixed_width/url")
            .and_then(Value::as_str)
            .or_else(|| images.pointer("/fixed_height/url").and_then(Value::as_str))
            .unwrap_or(url);
        // GIPHY publishes a static rendition next to each animation; without it the
        // hover-to-play previews would have nothing to show at rest.
        let still_url = images
            .pointer("/fixed_width_still/url")
            .and_then(Value::as_str)
            .or_else(|| images.pointer("/fixed_height_still/url").and_then(Value::as_str))
            .or_else(|| images.pointer("/downsized_still/url").and_then(Value::as_str))
            .filter(|still| !still.is_empty())
            .unwrap_or(preview_url);
        let width = images
            .pointer("/fixed_width/width")
            .and_then(Value::as_str)
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(0);
        let height = images
            .pointer("/fixed_width/height")
            .and_then(Value::as_str)
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(0);
        let title = item
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .unwrap_or(id);
        results.push(GifResult {
            id: id.to_string(),
            title: title.to_string(),
            url: url.to_string(),
            preview_url: preview_url.to_string(),
            still_url: still_url.to_string(),
            width,
            height,
        });
    }
    Ok(results)
}

/// Parses `url` and refuses anything that is not an HTTPS GIPHY media URL.
pub fn validate_url(url: &str) -> Result<reqwest::Url> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| AppError::msg("that is not a valid GIF address"))?;
    if parsed.scheme() != "https" {
        return Err(AppError::msg("GIF addresses must use HTTPS"));
    }
    let host = parsed
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| AppError::msg("that GIF address has no host"))?;
    if host != "giphy.com" && !host.ends_with(".giphy.com") {
        return Err(AppError::msg("only GIPHY GIF addresses can be copied"));
    }
    if !matches!(parsed.port(), None | Some(443)) {
        return Err(AppError::msg("GIF addresses must use the standard HTTPS port"));
    }
    Ok(parsed)
}

/// Downloads one GIF, returning its bytes.
pub async fn download(url: &str) -> Result<Vec<u8>> {
    validate_url(url)?;
    let response = client()?
        .get(url)
        .send()
        .await
        .map_err(|e| AppError::msg(format!("could not download the GIF: {e}")))?;
    let status = response.status();
    if !status.is_success() {
        return Err(AppError::msg(format!("GIPHY returned HTTP {status}")));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_GIF_BYTES as u64)
    {
        return Err(AppError::msg("that GIF is too large to copy"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| AppError::msg(format!("could not read the GIF: {e}")))?;
    if bytes.len() > MAX_GIF_BYTES {
        return Err(AppError::msg("that GIF is too large to copy"));
    }
    if !is_gif(&bytes) {
        return Err(AppError::msg("GIPHY did not return a GIF"));
    }
    Ok(bytes.to_vec())
}

fn is_gif(bytes: &[u8]) -> bool {
    bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")
}

/// Stores a downloaded GIF under `media/giphy/` so the clipboard can point at it.
///
/// The file has to outlive the clipboard write: pasting reads it lazily, so it is
/// kept in the data folder rather than a temp directory.
pub fn cache_file(media_dir: &Path, url: &str, bytes: &[u8]) -> Result<PathBuf> {
    let directory = media_dir.join("giphy");
    std::fs::create_dir_all(&directory)?;
    let path = directory.join(format!("{}.gif", hashing::of_str(url)));
    if !path.exists() {
        std::fs::write(&path, bytes)?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_giphy_media_urls() {
        assert!(validate_url("https://media.giphy.com/media/abc/giphy.gif").is_ok());
        assert!(validate_url("https://media3.giphy.com/media/abc/giphy.gif").is_ok());
        assert!(validate_url("https://giphy.com/gifs/abc").is_ok());
    }

    #[test]
    fn rejects_non_giphy_and_non_https_urls() {
        assert!(validate_url("http://media.giphy.com/media/abc/giphy.gif").is_err());
        assert!(validate_url("https://example.com/evil.gif").is_err());
        assert!(validate_url("https://giphy.com.evil.test/a.gif").is_err());
        assert!(validate_url("file:///C:/secret.gif").is_err());
        assert!(validate_url("https://media.giphy.com:8080/a.gif").is_err());
    }

    #[test]
    fn parses_a_search_response() {
        let payload = serde_json::json!({
            "meta": { "status": 200, "msg": "OK" },
            "data": [
                {
                    "id": "abc123",
                    "title": "Dancing cat",
                    "images": {
                        "original": { "url": "https://media.giphy.com/media/abc123/giphy.gif" },
                        "fixed_width": {
                            "url": "https://media.giphy.com/media/abc123/200w.gif",
                            "width": "200",
                            "height": "150"
                        },
                        "fixed_width_still": {
                            "url": "https://media.giphy.com/media/abc123/200w_s.gif"
                        }
                    }
                },
                {
                    "id": "no-images",
                    "title": "Skipped",
                    "images": {}
                }
            ]
        });
        let results = parse_payload(200, &payload).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Dancing cat");
        assert_eq!(results[0].width, 200);
        assert_eq!(results[0].preview_url, "https://media.giphy.com/media/abc123/200w.gif");
        assert_eq!(
            results[0].still_url,
            "https://media.giphy.com/media/abc123/200w_s.gif",
            "the static rendition is kept for hover-to-play"
        );
    }

    #[test]
    fn surfaces_the_api_error_message() {
        let payload = serde_json::json!({
            "meta": { "status": 403, "msg": "Invalid API key" }
        });
        let error = parse_payload(403, &payload).unwrap_err().to_string();
        assert!(error.contains("Invalid API key"));
    }

    #[test]
    fn a_missing_key_is_reported_instead_of_called() {
        let error = effective_key("   ").unwrap_err().to_string();
        assert!(error.contains("Settings"), "the message points at the setting");
        assert_eq!(effective_key(" personal ").unwrap(), "personal");
    }
}
