//! Link previews: the picture a page offers for itself.
//!
//! Nothing here runs unless the user asks for it — pressing **Load preview** on a
//! link clip, or turning on the setting that fetches one for every copied link.
//! CopyPaste is otherwise local-only, so the network is opt-in per clip.
//!
//! The fetch path reuses the guards the movie fetcher needs (HTTPS only, no
//! credentials, public addresses, same-host redirects) because a copied link is
//! just as untrusted as a copied image address. Only the first image the page
//! advertises is decoded, and it has to be a real image: a page that answers with
//! HTML gets no preview rather than a broken thumbnail.

use std::time::Duration;

use crate::error::{AppError, Result};

/// A page is text; anything this big is not a page worth previewing.
const MAX_PAGE_BYTES: usize = 2 * 1024 * 1024;
/// The image is kept as a thumbnail, so there is no point downloading a 12 MB
/// original only to shrink it.
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

/// What a page said about itself.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Preview {
    pub title: String,
    /// Absolute address of the image, if the page named one.
    pub image: Option<String>,
}

/// Fetches a page and reads the parts a preview card needs.
pub async fn load(url: &str) -> Result<(Preview, Vec<u8>)> {
    let page_url = crate::animated::validate_remote(url)?;
    let host = page_url
        .host_str()
        .ok_or_else(|| AppError::msg("that link has no host"))?
        .to_ascii_lowercase();

    let response = crate::animated::guarded_client(&host)?
        .get(page_url.clone())
        .header(
            reqwest::header::ACCEPT,
            "text/html,application/xhtml+xml;q=0.9,*/*;q=0.5",
        )
        .send()
        .await
        .map_err(|e| AppError::msg(format!("could not fetch that page: {e}")))?;
    if !response.status().is_success() {
        return Err(AppError::msg(format!(
            "the page returned HTTP {}",
            response.status()
        )));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_PAGE_BYTES as u64)
    {
        return Err(AppError::msg("that page is too large to preview"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| AppError::msg(format!("could not read that page: {e}")))?;
    if bytes.len() > MAX_PAGE_BYTES {
        return Err(AppError::msg("that page is too large to preview"));
    }
    let html = String::from_utf8_lossy(&bytes);
    let mut preview = parse(&html);
    let Some(image_url) = preview.image.take() else {
        return Err(AppError::msg("that page does not advertise a preview image"));
    };
    let image = fetch_image(&page_url, &image_url).await?;
    Ok((preview, image))
}

/// Fetches a page's preview picture and stores it on the clip.
///
/// Shared by the command and the optional automatic path in [`crate::clipboard`],
/// so both go through the same guards and the same thumbnail size.
pub async fn store_for(store: &crate::store::Store, id: i64, url: &str) -> Result<String> {
    let (_, image) = load(url).await?;
    let png = crate::media::preview_png(&image)?;
    store.attach_link_preview(id, &png)
}

/// Reads `<title>` and the first preview image a page advertises.
///
/// OpenGraph first, then Twitter's card, then a plain `<link rel="image_src">`:
/// the later forms are older, but they are what a page without OpenGraph tends
/// to have. Relative addresses are resolved against the page.
pub fn parse(html: &str) -> Preview {
    let title = tag_content(html, "title")
        .map(|text| decode_entities(text.trim()))
        .unwrap_or_default();
    let lower = html.to_ascii_lowercase();
    let open_graph = meta_content(html, &lower, "og:image")
        .or_else(|| meta_content(html, &lower, "og:image:url"))
        .or_else(|| meta_content(html, &lower, "twitter:image"))
        .or_else(|| link_rel_image(html, &lower));
    Preview {
        title,
        image: open_graph,
    }
}

/// Text between `<name ...>` and `</name>`, case-insensitively.
fn tag_content<'a>(html: &'a str, name: &str) -> Option<&'a str> {
    let lower = html.to_ascii_lowercase();
    let open = format!("<{name}");
    let start = lower.find(&open)?;
    let after_open = lower[start..].find('>')? + start + 1;
    let close = format!("</{name}");
    let end = lower[after_open..].find(&close)? + after_open;
    Some(&html[after_open..end])
}

/// The `content` of a `<meta property|name="..." content="...">` pair.
fn meta_content(html: &str, lower: &str, key: &str) -> Option<String> {
    let mut cursor = 0;
    while let Some(found) = lower[cursor..].find("<meta") {
        let start = cursor + found;
        let end = lower[start..].find('>').map(|offset| start + offset)?;
        let tag = &html[start..end];
        let name = attribute(tag, "property")
            .or_else(|| attribute(tag, "name"))
            .unwrap_or_default();
        if name.eq_ignore_ascii_case(key) {
            if let Some(value) = attribute(tag, "content") {
                let value = decode_entities(value.trim());
                if !value.is_empty() {
                    return Some(value);
                }
            }
        }
        cursor = end;
    }
    None
}

/// `<link rel="image_src" href="...">`, the format that predates OpenGraph.
fn link_rel_image(html: &str, lower: &str) -> Option<String> {
    let mut cursor = 0;
    while let Some(found) = lower[cursor..].find("<link") {
        let start = cursor + found;
        let end = lower[start..].find('>').map(|offset| start + offset)?;
        let tag = &html[start..end];
        if attribute(tag, "rel").is_some_and(|rel| rel.eq_ignore_ascii_case("image_src"))
            && let Some(href) = attribute(tag, "href")
        {
            let href = decode_entities(href.trim());
            if !href.is_empty() {
                return Some(href);
            }
        }
        cursor = end;
    }
    None
}

/// One attribute of a tag, quoted or unquoted.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    // Byte offsets come from the lowercased copy but slice the original: ASCII
    // lowercasing never changes a byte's length.
    let lower = tag.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut cursor = 0;
    while let Some(found) = lower[cursor..].find(name) {
        let start = cursor + found;
        // The name has to be a whole attribute, not the tail of `data-name`.
        let before_ok = start == 0
            || bytes[start - 1].is_ascii_whitespace()
            || matches!(bytes[start - 1], b'<' | b'=' | b'"' | b'\'');
        let after = start + name.len();
        let mut index = after;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if before_ok && index < bytes.len() && bytes[index] == b'=' {
            index += 1;
            while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            if index >= bytes.len() {
                return None;
            }
            let quote = bytes[index];
            if quote == b'"' || quote == b'\'' {
                let value_start = index + 1;
                let end = lower[value_start..].find(quote as char)? + value_start;
                return Some(&tag[value_start..end]);
            }
            let value_start = index;
            let end = lower[value_start..]
                .find(|c: char| c.is_ascii_whitespace() || c == '>')
                .map(|offset| value_start + offset)
                .unwrap_or(lower.len());
            return Some(&tag[value_start..end]);
        }
        cursor = after;
    }
    None
}

/// The handful of entities a title or URL actually uses.
fn decode_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
}

/// Downloads the image a page pointed at.
///
/// The image may live on another host (a CDN is the usual case), so it is
/// resolved against the page and then checked on its own merits.
async fn fetch_image(page: &reqwest::Url, raw: &str) -> Result<Vec<u8>> {
    let url = page
        .join(raw)
        .map_err(|_| AppError::msg("that preview image address is not valid"))?;
    let host = url
        .host_str()
        .ok_or_else(|| AppError::msg("that preview image has no host"))?
        .to_ascii_lowercase();
    let response = crate::animated::guarded_client(&host)?
        .get(url.clone())
        .header(reqwest::header::ACCEPT, "image/*")
        .send()
        .await
        .map_err(|e| AppError::msg(format!("could not fetch that preview image: {e}")))?;
    if !response.status().is_success() {
        return Err(AppError::msg(format!(
            "the image host returned HTTP {}",
            response.status()
        )));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_IMAGE_BYTES as u64)
    {
        return Err(AppError::msg("that preview image is too large"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| AppError::msg(format!("could not read that preview image: {e}")))?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(AppError::msg("that preview image is too large"));
    }
    Ok(bytes.to_vec())
}

/// Resolves the preview image against its page, so the caller stores an absolute
/// address. Only the tests need it, which never touch the network.
#[cfg(test)]
pub fn resolve_image(page: &str, raw: &str) -> Option<String> {
    reqwest::Url::parse(page)
        .ok()?
        .join(raw)
        .ok()
        .map(|url| url.to_string())
}

/// True when the page's own address is one the fetcher would accept.
pub fn is_fetchable(url: &str) -> bool {
    crate::animated::validate_remote(url).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_open_graph_title_and_image() {
        let html = r#"<html><head><title>A page &amp; its title</title>
            <meta property="og:title" content="ignored">
            <meta property="og:image" content="https://cdn.example.com/shot.png">
            </head><body>hello</body></html>"#;
        let preview = parse(html);
        assert_eq!(preview.title, "A page & its title");
        assert_eq!(
            preview.image.as_deref(),
            Some("https://cdn.example.com/shot.png")
        );
    }

    #[test]
    fn falls_back_to_twitter_then_to_image_src() {
        let twitter = r#"<meta name="twitter:image" content="https://cdn.example.com/tw.png">"#;
        assert_eq!(
            parse(twitter).image.as_deref(),
            Some("https://cdn.example.com/tw.png")
        );

        let legacy = r#"<link rel="image_src" href="https://cdn.example.com/old.png">"#;
        assert_eq!(
            parse(legacy).image.as_deref(),
            Some("https://cdn.example.com/old.png")
        );
        assert_eq!(parse("<p>nothing here</p>").image, None);
    }

    #[test]
    fn an_og_image_beats_the_other_forms() {
        let html = r#"<link rel="image_src" href="https://cdn.example.com/old.png">
            <meta name="twitter:image" content="https://cdn.example.com/tw.png">
            <meta property="og:image" content="https://cdn.example.com/best.png">"#;
        assert_eq!(
            parse(html).image.as_deref(),
            Some("https://cdn.example.com/best.png")
        );
    }

    #[test]
    fn attributes_are_read_quoted_unescaped_or_single_quoted_whatever_the_case() {
        let html = r#"<META PROPERTY='OG:IMAGE' CONTENT=https://cdn.example.com/a.png?x=1&amp;y=2>"#;
        assert_eq!(
            parse(html).image.as_deref(),
            Some("https://cdn.example.com/a.png?x=1&y=2")
        );
    }

    #[test]
    fn a_data_name_is_not_mistaken_for_its_name() {
        let html = r#"<meta data-content="nope" name="og:image" content="https://cdn.example.com/right.png">"#;
        assert_eq!(
            parse(html).image.as_deref(),
            Some("https://cdn.example.com/right.png")
        );
    }

    #[test]
    fn a_relative_image_is_resolved_against_the_page() {
        assert_eq!(
            resolve_image("https://example.com/posts/one", "/img/cover.png").as_deref(),
            Some("https://example.com/img/cover.png")
        );
        assert_eq!(
            resolve_image("https://example.com/posts/one", "cover.png").as_deref(),
            Some("https://example.com/posts/cover.png")
        );
    }

    #[test]
    fn only_addresses_the_fetcher_accepts_are_previewed() {
        assert!(is_fetchable("https://example.com/a"));
        assert!(!is_fetchable("http://example.com/a"));
        assert!(!is_fetchable("https://localhost/a"));
        assert!(!is_fetchable("not a link"));
    }
}
