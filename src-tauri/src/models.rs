//! Domain types shared by storage, capture and the IPC layer.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipKind {
    Text,
    Image,
    Files,
}

impl ClipKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            ClipKind::Text => "text",
            ClipKind::Image => "image",
            ClipKind::Files => "files",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "text" => Some(ClipKind::Text),
            "image" => Some(ClipKind::Image),
            "files" => Some(ClipKind::Files),
            _ => None,
        }
    }
}

/// A decoded clipboard payload, before it is hashed and persisted.
#[derive(Debug, Clone)]
pub enum ClipContent {
    Text(String),
    Image {
        width: u32,
        height: u32,
        /// Tightly packed RGBA8, `width * height * 4` bytes.
        rgba: Vec<u8>,
    },
    Files(Vec<String>),
}

impl ClipContent {
    pub const fn kind(&self) -> ClipKind {
        match self {
            ClipContent::Text(_) => ClipKind::Text,
            ClipContent::Image { .. } => ClipKind::Image,
            ClipContent::Files(_) => ClipKind::Files,
        }
    }
}

/// A row of `clips` exactly as stored, i.e. with ciphertext still sealed.
#[derive(Debug, Clone)]
pub struct ClipRow {
    pub id: i64,
    pub kind: ClipKind,
    pub byte_size: i64,
    pub char_count: i64,
    pub file_count: i64,
    pub preview: Option<Vec<u8>>,
    pub preview_nonce: Option<Vec<u8>>,
    pub body: Option<Vec<u8>>,
    pub body_nonce: Option<Vec<u8>>,
    pub files: Option<Vec<u8>>,
    pub files_nonce: Option<Vec<u8>>,
    pub image_path: Option<String>,
    pub thumb_path: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub source_app: Option<String>,
    pub pinned: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub copy_count: i64,
    /// Recognised text, sealed like the other payload columns.
    pub ocr_text: Option<Vec<u8>>,
    pub ocr_nonce: Option<Vec<u8>>,
    /// Rich flavours captured alongside the plain body, sealed like it.
    pub html: Option<Vec<u8>>,
    pub html_nonce: Option<Vec<u8>>,
    pub rtf: Option<Vec<u8>>,
    pub rtf_nonce: Option<Vec<u8>>,
    /// A user-authored note, sealed like the payload columns.
    pub note: Option<Vec<u8>>,
    pub note_nonce: Option<Vec<u8>>,
    /// Credential confidence from capture time; plaintext on purpose, so the list
    /// can warn about a clip without decrypting it.
    pub secret_level: Option<String>,
}

/// What the history list needs. Deliberately excludes the full body so that
/// rendering a few hundred rows never ships megabytes of text over IPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipSummary {
    pub id: i64,
    pub kind: ClipKind,
    pub preview: String,
    pub byte_size: i64,
    pub source_app: Option<String>,
    pub pinned: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub copy_count: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    /// Media file names served by the `media` URI scheme; the frontend turns these
    /// into real URLs with `convertFileSrc`. `None` for non-image clips.
    pub thumb_file: Option<String>,
    pub image_file: Option<String>,
    pub file_count: i64,
    pub char_count: i64,
    /// True when an image clip has recognised, non-empty text.
    pub has_ocr: bool,
    /// True when the user attached a note to the clip.
    pub has_note: bool,
    /// For text clips whose whole payload is a color code: the normalised
    /// `#rrggbb` value. Purely derived, never stored.
    pub color: Option<String>,
    /// Derived shape of the payload: "color", "url", "email", "path" or "json".
    pub content_kind: Option<String>,
    /// Absolute path of the stored image, so the UI can hand it to Explorer or a
    /// drag-and-drop target without knowing where the data folder lives.
    pub image_path: Option<String>,
    /// True when an animated GIF was recovered for this image clip, so copying it
    /// publishes the file rather than the rasterised first frame.
    pub animated: bool,
    pub collection_ids: Vec<i64>,
    pub tags: Vec<String>,
    /// Confidence that the payload holds a credential: "low", "medium" or "high".
    /// Stored in plaintext so the list can warn without decrypting the clip.
    pub secret_level: Option<String>,
}

/// A summary plus the payload needed to actually use the clip.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipDetail {
    #[serde(flatten)]
    pub summary: ClipSummary,
    /// Present for text clips.
    pub text: Option<String>,
    /// Present for file clips.
    pub file_paths: Vec<String>,
    /// Text recognised in an image clip, when there is any.
    pub ocr_text: Option<String>,
    /// A free-form note the user attached to the clip.
    pub note: Option<String>,
    /// True when the clip carries HTML/RTF captured next to its plain text.
    pub has_rich: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RichText {
    pub html: Option<String>,
    pub rtf: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipPage {
    pub items: Vec<ClipSummary>,
    /// Number of clips matching the filter irrespective of paging.
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    /// Archive collections keep their clips out of the ordinary history views and
    /// out of every bulk delete.
    pub archived: bool,
    pub clip_count: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub clip_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub total: i64,
    pub pinned: i64,
    pub images: i64,
    pub files: i64,
    /// Clips that only live in an archive collection, so the history counts above
    /// can stay a picture of the active window.
    pub archived: i64,
    /// Text clips whose payload is a single color code.
    pub colors: i64,
    pub media_bytes: i64,
    pub database_bytes: i64,
    pub encryption: String,
}

/// Which clips a query should consider. Mirrors the sidebar selection in the UI.
///
/// `default` matters here: the frontend sends partial filters such as `{}` or
/// `{ "kind": "text" }`, and without it serde would demand every field, making
/// every list request fail.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClipFilter {
    /// Restrict to a single kind.
    pub kind: Option<ClipKind>,
    /// Restrict to clips filed under this collection.
    pub collection_id: Option<i64>,
    /// Restrict to clips carrying this tag.
    pub tag: Option<String>,
    /// Restrict to pinned clips.
    pub pinned_only: bool,
    /// Restrict to text clips that are a single color code.
    pub colors_only: bool,
    /// Reach clips filed in archive collections, which are hidden by default.
    pub include_archived: bool,
    /// Case-insensitive substring match against the source application.
    pub app: Option<String>,
    /// Only clips captured before this Unix-millisecond timestamp.
    pub before: Option<i64>,
    /// Exact color match, for the `color:` search operator.
    pub color: Option<String>,
    /// Derived payload shape ("url", "email", "path", "json"), for `type:`.
    pub content_kind: Option<String>,
    /// Restrict to clips flagged as credential-like.
    pub secrets_only: bool,
}

/// Insert payload produced by capture.
#[derive(Debug, Clone)]
pub struct NewClip {
    pub content: ClipContent,
    pub content_hash: String,
    pub source_app: Option<String>,
    /// Lowercased text used only by the in-memory search index.
    pub search_text: String,
    pub preview: String,
    /// Rich flavours offered by the source application alongside plain text.
    pub html: Option<String>,
    pub rtf: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The UI sends partial filters (`{}` for "All clips", `{"kind":"text"}` for a
    /// sidebar entry). If `ClipFilter` ever loses `#[serde(default)]` every list
    /// request fails at runtime with "missing field", which no compiler catches.
    #[test]
    fn partial_filters_deserialize() {
        let empty: ClipFilter = serde_json::from_str("{}").expect("an empty filter must be valid");
        assert!(empty.kind.is_none());
        assert!(!empty.pinned_only);

        let by_kind: ClipFilter =
            serde_json::from_str(r#"{"kind":"image"}"#).expect("kind-only filter");
        assert_eq!(by_kind.kind, Some(ClipKind::Image));
        assert!(!by_kind.pinned_only);

        let pinned: ClipFilter =
            serde_json::from_str(r#"{"pinnedOnly":true}"#).expect("pinned-only filter");
        assert!(pinned.pinned_only);

        let in_collection: ClipFilter =
            serde_json::from_str(r#"{"collectionId":7}"#).expect("collection filter");
        assert_eq!(in_collection.collection_id, Some(7));

        let tagged: ClipFilter = serde_json::from_str(r#"{"tag":"ops"}"#).expect("tag filter");
        assert_eq!(tagged.tag.as_deref(), Some("ops"));
    }

    #[test]
    fn filters_serialize_with_camel_case_keys() {
        let json = serde_json::to_string(&ClipFilter {
            pinned_only: true,
            collection_id: Some(3),
            ..Default::default()
        })
        .unwrap();
        assert!(json.contains("\"pinnedOnly\""), "got {json}");
        assert!(json.contains("\"collectionId\""), "got {json}");
    }

    #[test]
    fn clip_kind_round_trips_through_serde() {
        for kind in [ClipKind::Text, ClipKind::Image, ClipKind::Files] {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(serde_json::from_str::<ClipKind>(&json).unwrap(), kind);
        }
        assert_eq!(serde_json::to_string(&ClipKind::Files).unwrap(), "\"files\"");
    }
}
