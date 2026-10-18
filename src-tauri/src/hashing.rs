//! Content identity, used to de-duplicate the clipboard history.
//!
//! A SHA-256 digest of the payload is stored in plaintext and doubles as the
//! unique key on `clips`, so copying the same thing twice promotes one row
//! instead of appending a duplicate. Storing the digest unencrypted is a
//! deliberate trade: it is one-way, but equal payloads remain detectable to
//! someone reading the database. De-duplication is impossible without that.
//!
//! Each kind is prefixed with a domain separator so that, for example, the text
//! `"a.txt"` cannot collide with a file clip holding the single path `a.txt`.

use sha2::{Digest, Sha256};

use crate::models::ClipContent;

const TEXT_DOMAIN: &[u8] = b"copypaste:text\0";
const IMAGE_DOMAIN: &[u8] = b"copypaste:image\0";
const FILES_DOMAIN: &[u8] = b"copypaste:files\0";

pub fn of_str(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(TEXT_DOMAIN);
    hasher.update(value.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn of_content(content: &ClipContent) -> String {
    match content {
        ClipContent::Text(text) => of_str(text),
        ClipContent::Image {
            width,
            height,
            rgba,
        } => {
            // Hash the decoded pixels, not the encoded PNG: the same image can be
            // offered by the clipboard with different byte-level encodings.
            let mut hasher = Sha256::new();
            hasher.update(IMAGE_DOMAIN);
            hasher.update(width.to_le_bytes());
            hasher.update(height.to_le_bytes());
            hasher.update(rgba);
            hex::encode(hasher.finalize())
        }
        ClipContent::Files(paths) => {
            let mut hasher = Sha256::new();
            hasher.update(FILES_DOMAIN);
            for path in paths {
                // Case-folded because Windows paths that differ only in case refer
                // to the same file.
                hasher.update(path.to_lowercase().as_bytes());
                hasher.update([0u8]);
            }
            hex::encode(hasher.finalize())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(seed: u8) -> ClipContent {
        ClipContent::Image {
            width: 2,
            height: 2,
            rgba: vec![seed; 16],
        }
    }

    #[test]
    fn identical_payloads_hash_identically() {
        assert_eq!(of_str("same"), of_str("same"));
        assert_eq!(of_content(&image(9)), of_content(&image(9)));
        assert_eq!(
            of_content(&ClipContent::Files(vec!["C:\\a.txt".into()])),
            of_content(&ClipContent::Files(vec!["C:\\a.txt".into()])),
        );
    }

    #[test]
    fn different_payloads_hash_differently() {
        assert_ne!(of_str("a"), of_str("b"));
        assert_ne!(of_content(&image(1)), of_content(&image(2)));
    }

    #[test]
    fn digests_are_hex_sha256() {
        let digest = of_str("anything");
        assert_eq!(digest.len(), 64);
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn kinds_cannot_collide_across_domains() {
        let text = ClipContent::Text("a.txt".into());
        let files = ClipContent::Files(vec!["a.txt".into()]);
        assert_ne!(of_content(&text), of_content(&files));
    }

    #[test]
    fn file_hashes_are_case_insensitive() {
        let lower = ClipContent::Files(vec!["c:\\temp\\a.txt".into()]);
        let upper = ClipContent::Files(vec!["C:\\TEMP\\A.TXT".into()]);
        assert_eq!(of_content(&lower), of_content(&upper));
    }

    #[test]
    fn image_hashes_depend_on_dimensions_and_pixels() {
        let base = image(3);
        let wider = ClipContent::Image {
            width: 4,
            height: 2,
            rgba: vec![3; 32],
        };
        assert_ne!(of_content(&base), of_content(&wider));

        // Same pixel count, different order.
        let reordered = ClipContent::Image {
            width: 2,
            height: 2,
            rgba: vec![4, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3],
        };
        assert_ne!(of_content(&base), of_content(&reordered));
    }

    #[test]
    fn file_order_matters() {
        let ab = ClipContent::Files(vec!["a".into(), "b".into()]);
        let ba = ClipContent::Files(vec!["b".into(), "a".into()]);
        assert_ne!(of_content(&ab), of_content(&ba));
        // And paths cannot be confused by concatenation ("ab" vs "a","b").
        let joined = ClipContent::Files(vec!["ab".into()]);
        assert_ne!(of_content(&ab), of_content(&joined));
    }
}
