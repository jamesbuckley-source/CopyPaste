//! Legacy record decryption, used only to migrate histories written before
//! CopyPaste stored clips as plain text.
//!
//! Older builds sealed sensitive columns with AES-256-GCM under a random key that
//! was itself wrapped with Windows DPAPI (`CryptProtectData`). Encryption has been
//! removed, so there is no key generation and no sealing API in release builds:
//! startup decrypts any legacy rows once, rewrites them as plain text, and deletes
//! `key.bin`. The test-only seal helper keeps the decryption path covered.

use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit};
#[cfg(test)]
use aes_gcm::aead::{OsRng, rand_core::RngCore};
use aes_gcm::{Aes256Gcm, Nonce};
use zeroize::Zeroize;

use crate::error::{AppError, Result};

pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 12;

/// The unwrapped data key. Zeroed on drop so it does not linger in freed memory.
pub struct MasterKey([u8; KEY_LEN]);

impl MasterKey {
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

/// Reads a legacy wrapped key. Missing keys are not generated any more.
pub fn load_master_key(key_path: &Path) -> Result<MasterKey> {
    let sealed = std::fs::read(key_path)?;
    let raw = unprotect(&sealed)?;
    MasterKey::from_slice(&raw)
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

    #[cfg(test)]
    pub fn open_str(&self, nonce: &[u8], ciphertext: &[u8]) -> Result<String> {
        let bytes = self.open(nonce, ciphertext)?;
        String::from_utf8(bytes)
            .map_err(|_| AppError::Crypto("decrypted record is not valid UTF-8".into()))
    }
}

/// The legacy ciphertext shape, kept for the migration tests.
#[cfg(test)]
pub struct Sealed {
    pub nonce: [u8; NONCE_LEN],
    pub ciphertext: Vec<u8>,
}

#[cfg(test)]
impl Cipher {
    pub fn seal_for_test(&self, plain: &str) -> Result<Sealed> {
        let mut nonce = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce);
        let ciphertext = self
            .inner
            .encrypt(Nonce::from_slice(&nonce), plain.as_bytes())
            .map_err(|_| AppError::Crypto("failed to encrypt test record".into()))?;
        Ok(Sealed { nonce, ciphertext })
    }
}

#[cfg(test)]
pub fn test_key() -> MasterKey {
    MasterKey([7u8; KEY_LEN])
}

#[cfg(test)]
pub fn write_test_key(key: &MasterKey, path: &Path) -> Result<()> {
    std::fs::write(path, protect(key.as_bytes())?)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Windows DPAPI key wrapping
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod platform {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptUnprotectData,
    };
    #[cfg(test)]
    use windows::Win32::Security::Cryptography::CryptProtectData;

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

    #[cfg(test)]
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

    /// DPAPI is Windows-only, and no new keys are written; a non-Windows build
    /// could not unwrap a legacy key file.
    pub fn unprotect(_data: &[u8]) -> Result<Vec<u8>> {
        Err(AppError::Crypto(
            "legacy key unwrapping requires Windows DPAPI".into(),
        ))
    }

    #[cfg(test)]
    pub fn protect(_data: &[u8]) -> Result<Vec<u8>> {
        Err(AppError::Crypto(
            "test key wrapping requires Windows DPAPI".into(),
        ))
    }
}

use platform::unprotect;
#[cfg(test)]
use platform::protect;

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
    fn legacy_roundtrip_preserves_utf8() {
        let cipher = test_cipher();
        let original = "hello clipboard — ünïcödé ✓ 日本語";
        let sealed = cipher.seal_for_test(original).unwrap();
        assert_ne!(sealed.ciphertext, original.as_bytes());
        assert_eq!(
            cipher.open_str(&sealed.nonce, &sealed.ciphertext).unwrap(),
            original
        );
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let cipher = test_cipher();
        let mut sealed = cipher.seal_for_test("authentic").unwrap();
        let last = sealed.ciphertext.len() - 1;
        sealed.ciphertext[last] ^= 0x01;
        assert!(
            cipher.open_str(&sealed.nonce, &sealed.ciphertext).is_err(),
            "GCM must detect modification"
        );
    }

    #[test]
    fn wrong_key_cannot_decrypt() {
        let sealed = test_cipher().seal_for_test("secret").unwrap();
        let mut other = [0u8; KEY_LEN];
        other[0] = 0xFF;
        let wrong = Cipher::new(&MasterKey(other));
        assert!(wrong.open_str(&sealed.nonce, &sealed.ciphertext).is_err());
    }

    #[test]
    fn short_nonce_is_rejected() {
        let cipher = test_cipher();
        let sealed = cipher.seal_for_test("x").unwrap();
        assert!(cipher.open(&sealed.nonce[..4], &sealed.ciphertext).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn legacy_key_survives_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("key.bin");

        let first = test_key();
        write_test_key(&first, &key_path).unwrap();
        let second = load_master_key(&key_path).unwrap();
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
        assert!(load_master_key(&key_path).is_err());
    }
}
