//! Central error type shared across backend modules.
//!
//! `AppError` is the single error surface crossing the IPC boundary, so it also
//! implements `Serialize`: Tauri rejects `#[tauri::command]` return types whose
//! error does not serialize, and the frontend receives a plain string.

use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("image error: {0}")]
    Image(#[from] image::ImageError),

    #[error("tauri error: {0}")]
    Tauri(#[from] tauri::Error),

    #[error("clipboard error: {0}")]
    Clipboard(String),

    #[error("legacy decryption error: {0}")]
    Crypto(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("{0}")]
    Message(String),
}

impl AppError {
    pub fn msg(message: impl Into<String>) -> Self {
        AppError::Message(message.into())
    }
}

/// Lets `?` work directly on Win32 calls inside clipboard and paste helpers.
#[cfg(windows)]
impl From<windows::core::Error> for AppError {
    fn from(error: windows::core::Error) -> Self {
        AppError::Clipboard(error.to_string())
    }
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
