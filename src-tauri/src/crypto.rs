//! Data-at-rest encryption.
//!
//! Two layers, deliberately kept separate:
//!
//! 1. **Key wrapping** — a random 256-bit data key is generated once and sealed
//!    with Windows DPAPI (`CryptProtectData`), which derives its protection from
//!    the logged-on user's credentials. The sealed blob is what lands on disk, so
//!    an attacker who copies `key.bin` to another machine or user account cannot
//!    unwrap it. This is why there is no passphrase prompt.
//!
//! 2. **Record encryption** — every sensitive column is sealed individually with
//!    AES-256-GCM. GCM is authenticated, so tampering with a ciphertext blob in
//!    the database is detected on read rather than silently decrypted to garbage.
//!
//! A fresh random nonce is generated per record. Nonce reuse is the classic
//! failure mode for GCM, and generating them randomly per record over a 96-bit
//! space is safe for the record counts a clipboard history can reach.

use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit, OsRng, rand_core::RngCore};
use aes_gcm::{Aes256Gcm, Nonce};
use zeroize::Zeroize;

use crate::error::{AppError, Result};

pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 12;

/// The unwrapped data key. Zeroed on drop so it does not linger in freed memory.
pub struct MasterKey([u8; KEY_LEN]);

impl MasterKey {
    fn generate() -> Self {
        let mut bytes = [0u8; KEY_LEN];
        OsRng.fill_bytes(&mut bytes);
        MasterKey(bytes)
    }

    fn from_slice(raw: &[u8]) -> Result<Self> {
        if raw.len() != KEY_LEN {
            return Err(AppError::Crypto(format!(
                "expected a {KEY_LEN}-byte key, found {} bytes",
                raw.len()
            )));
        }
        let mut bytes = [0u8; KEY_LEN];
        bytes.copy_from_slice(raw);
        Ok(MasterKey(bytes))
    }

    fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

impl Drop for MasterKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Reads the sealed data key, generating and persisting one on first run.
pub fn load_or_create_master_key(key_path: &Path) -> Result<MasterKey> {
    if key_path.exists() {
        let sealed = std::fs::read(key_path)?;
        let raw = unprotect(&sealed)?;
        MasterKey::from_slice(&raw)
    } else {
        let key = MasterKey::generate();
        let sealed = protect(key.as_bytes())?;
        if let Some(parent) = key_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_atomic(key_path, &sealed)?;
        Ok(key)
    }
}

/// Writes via a temp file + rename so a crash mid-write cannot leave a truncated
/// key file behind (which would make every stored record unreadable).
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct Sealed {
    pub nonce: [u8; NONCE_LEN],
    pub ciphertext: Vec<u8>,
}

pub struct Cipher {
    inner: Aes256Gcm,
}

impl Cipher {
    pub fn new(key: &MasterKey) -> Self {
        let inner = Aes256Gcm::new_from_slice(key.as_bytes())
            .expect("AES-256-GCM accepts exactly 32-byte keys");
        Cipher { inner }
    }

    pub fn seal(&self, plain: &[u8]) -> Result<Sealed> {
        let mut nonce = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce);
        let ciphertext = self
            .inner
            .encrypt(Nonce::from_slice(&nonce), plain)
            .map_err(|_| AppError::Crypto("failed to encrypt record".into()))?;
        Ok(Sealed { nonce, ciphertext })
    }

    pub fn open(&self, nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>> {
        if nonce.len() != NONCE_LEN {
            return Err(AppError::Crypto(format!(
                "malformed nonce: expected {NONCE_LEN} bytes, found {}",
                nonce.len()
            )));
        }
        self.inner
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .map_err(|_| {
                AppError::Crypto("failed to decrypt record (wrong key or tampered data)".into())
            })
    }

    pub fn seal_str(&self, plain: &str) -> Result<Sealed> {
        self.seal(plain.as_bytes())
    }

    pub fn open_str(&self, nonce: &[u8], ciphertext: &[u8]) -> Result<String> {
        let bytes = self.open(nonce, ciphertext)?;
        String::from_utf8(bytes)
            .map_err(|_| AppError::Crypto("decrypted record is not valid UTF-8".into()))
    }
}

// ---------------------------------------------------------------------------
// Windows DPAPI key wrapping
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod platform {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };

    use super::{AppError, Result};

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            // DPAPI only reads from the input blob; the cast mirrors the Win32
            // signature, which is not const-correct here.
            pbData: data.as_ptr().cast_mut(),
        }
    }

    /// # Safety
    /// `out` must be a DPAPI-allocated blob that is still owned by the caller.
    unsafe fn take_blob(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec() };
        let _ = unsafe { LocalFree(Some(HLOCAL(out.pbData.cast()))) };
        bytes
    }

    pub fn protect(data: &[u8]) -> Result<Vec<u8>> {
        let input = blob(data);
        let mut output = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptProtectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
            .map_err(|e| AppError::Crypto(format!("DPAPI protect failed: {e}")))?;
            Ok(take_blob(output))
        }
    }

    pub fn unprotect(data: &[u8]) -> Result<Vec<u8>> {
        let input = blob(data);
        let mut output = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
            .map_err(|e| {
                AppError::Crypto(format!(
                    "DPAPI unprotect failed: {e}. The key file can only be read by the \
                     Windows user account that created it."
                ))
            })?;
            Ok(take_blob(output))
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{AppError, Result};

    /// DPAPI is Windows-only. Failing loudly is preferable to silently writing an
    /// unprotected key on a platform where the security guarantee would not hold.
    pub fn protect(_data: &[u8]) -> Result<Vec<u8>> {
        Err(AppError::Crypto(
            "key wrapping requires Windows DPAPI; this build targets Windows only".into(),
        ))
    }

    pub fn unprotect(_data: &[u8]) -> Result<Vec<u8>> {
        Err(AppError::Crypto(
            "key unwrapping requires Windows DPAPI; this build targets Windows only".into(),
        ))
    }
}

use platform::{protect, unprotect};

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cipher() -> Cipher {
        let mut raw = [0u8; KEY_LEN];
        for (i, b) in raw.iter_mut().enumerate() {
            *b = i as u8;
        }
        Cipher::new(&MasterKey(raw))
    }

    #[test]
    fn seal_open_roundtrip_preserves_utf8() {
        let cipher = test_cipher();
        let original = "hello clipboard — ünïcödé ✓ 日本語";
        let sealed = cipher.seal_str(original).unwrap();
        assert_ne!(sealed.ciphertext, original.as_bytes());
        assert_eq!(
            cipher.open_str(&sealed.nonce, &sealed.ciphertext).unwrap(),
            original
        );
    }

    #[test]
    fn every_seal_uses_a_distinct_nonce() {
        let cipher = test_cipher();
        let a = cipher.seal_str("same input").unwrap();
        let b = cipher.seal_str("same input").unwrap();
        assert_ne!(a.nonce, b.nonce, "nonce reuse would break GCM");
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let cipher = test_cipher();
        let mut sealed = cipher.seal_str("authentic").unwrap();
        let last = sealed.ciphertext.len() - 1;
        sealed.ciphertext[last] ^= 0x01;
        assert!(
            cipher.open_str(&sealed.nonce, &sealed.ciphertext).is_err(),
            "GCM must detect modification"
        );
    }

    #[test]
    fn wrong_key_cannot_decrypt() {
        let sealed = test_cipher().seal_str("secret").unwrap();
        let mut other = [0u8; KEY_LEN];
        other[0] = 0xFF;
        let wrong = Cipher::new(&MasterKey(other));
        assert!(wrong.open_str(&sealed.nonce, &sealed.ciphertext).is_err());
    }

    #[test]
    fn short_nonce_is_rejected() {
        let cipher = test_cipher();
        let sealed = cipher.seal_str("x").unwrap();
        assert!(cipher.open(&sealed.nonce[..4], &sealed.ciphertext).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_roundtrip_and_generated_key_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("key.bin");

        let first = load_or_create_master_key(&key_path).unwrap();
        assert!(key_path.exists());

        // The key must survive a reload, and the on-disk form must not be the raw key.
        let second = load_or_create_master_key(&key_path).unwrap();
        let on_disk = std::fs::read(&key_path).unwrap();
        assert_eq!(first.as_bytes(), second.as_bytes());
        assert_ne!(on_disk, first.as_bytes().to_vec());
        assert!(!on_disk.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn corrupted_key_file_is_reported_not_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("key.bin");
        std::fs::write(&key_path, b"this is not a DPAPI blob").unwrap();
        assert!(load_or_create_master_key(&key_path).is_err());
    }
}
