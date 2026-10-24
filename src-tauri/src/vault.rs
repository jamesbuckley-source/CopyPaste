//! The vault: the passphrase that protects clips marked secret.
//!
//! Two keys, not one. A random 32-byte data key encrypts every secret row, and the
//! passphrase only protects *that* key. Changing the passphrase therefore rewraps
//! 32 bytes instead of re-encrypting the history, and a copied database stays
//! openable anywhere because nothing here is bound to this machine: the salt and
//! the wrapped key travel inside the same database.
//!
//! The passphrase is stretched with Argon2id — memory-hard on purpose, because a
//! copied file is exactly the threat, and a GPU-friendly KDF would make guessing a
//! human passphrase cheap. What this does **not** protect against is a machine
//! that is compromised while the app is running and unlocked; that is stated in
//! the interface rather than pretended away.

use aes_gcm::aead::{Aead, KeyInit, OsRng, rand_core::RngCore};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::{AppError, Result};

/// Length of the key that actually encrypts rows.
const DATA_KEY_LEN: usize = 32;
/// AES-GCM's nonce length. Random per row: at this volume a collision is not a
/// practical concern, and a counter would have to be stored and trusted.
const NONCE_LEN: usize = 12;
/// A fixed plaintext whose recovery proves the passphrase was right, without
/// having to touch (and therefore leak the existence of) any real clip.
const CHECK_PLAINTEXT: &[u8] = b"copypaste-vault-check";

/// Argon2id cost. Roughly a quarter of a second on a modern desktop: slow enough
/// that a stolen file is expensive to attack, fast enough that unlocking feels
/// immediate.
const MEMORY_KIB: u32 = 65_536;
const ITERATIONS: u32 = 3;
const PARALLELISM: u32 = 1;

/// Everything needed to reopen the vault, stored beside the clips.
///
/// `version` exists so a future change of parameters or cipher can be detected
/// rather than misread as a corrupt file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultMeta {
    pub version: u32,
    /// Base64 Argon2 salt, regenerated whenever the passphrase changes.
    pub salt: String,
    /// Base64 wrapped data key.
    pub wrapped: String,
    /// Base64 of [`CHECK_PLAINTEXT`] sealed under the wrapped key.
    pub check: String,
}

/// The data key, wiped when it goes out of scope.
pub struct VaultKey(Zeroizing<[u8; DATA_KEY_LEN]>);

impl VaultKey {
    fn cipher(&self) -> Result<Aes256Gcm> {
        Aes256Gcm::new_from_slice(self.0.as_slice())
            .map_err(|_| AppError::msg("the vault key is not a valid key"))
    }

    /// Encrypts one value: a fresh nonce followed by the sealed bytes.
    ///
    /// The nonce travels with the ciphertext, so a row needs no second column and a
    /// database copied elsewhere carries everything required to read it back.
    pub fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let cipher = self.cipher()?;
        let mut nonce = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce);
        let sealed = cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext)
            .map_err(|_| AppError::msg("could not encrypt that value"))?;
        let mut blob = Vec::with_capacity(NONCE_LEN + sealed.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&sealed);
        Ok(blob)
    }

    /// The inverse of [`VaultKey::seal`]. A tampered or truncated blob fails here
    /// rather than decrypting to nonsense.
    pub fn open(&self, blob: &[u8]) -> Result<Vec<u8>> {
        if blob.len() <= NONCE_LEN {
            return Err(AppError::msg("that encrypted value is incomplete"));
        }
        let (nonce, sealed) = blob.split_at(NONCE_LEN);
        let cipher = self.cipher()?;
        cipher
            .decrypt(Nonce::from_slice(nonce), sealed)
            .map_err(|_| AppError::msg("that encrypted value could not be read"))
    }

    pub fn seal_text(&self, text: &str) -> Result<Vec<u8>> {
        self.seal(text.as_bytes())
    }

    pub fn open_text(&self, blob: &[u8]) -> Result<String> {
        String::from_utf8(self.open(blob)?)
            .map_err(|_| AppError::msg("that encrypted value is not text"))
    }

    /// Base64 of the data key, for the one place that has to store it wrapped.
    fn to_b64(&self) -> String {
        base64(self.0.as_slice())
    }

    fn from_b64(text: &str) -> Result<Self> {
        let raw = unbase64(text)?;
        let bytes: [u8; DATA_KEY_LEN] = raw
            .try_into()
            .map_err(|_| AppError::msg("the wrapped key has the wrong length"))?;
        Ok(Self(Zeroizing::new(bytes)))
    }
}

/// Creates a vault: a new random data key, wrapped by a key derived from
/// `passphrase`.
pub fn create(passphrase: &str) -> Result<(VaultMeta, VaultKey)> {
    let mut data_key = [0u8; DATA_KEY_LEN];
    OsRng.fill_bytes(&mut data_key);
    let key = VaultKey(Zeroizing::new(data_key));
    let meta = wrap(passphrase, &key)?;
    Ok((meta, key))
}

/// Rewraps the same data key under a new passphrase.
///
/// Nothing but the wrapped key changes, which is the whole reason the design has
/// two keys: the clips are never re-encrypted, and no row can be left half-done.
pub fn rewrap(passphrase: &str, key: &VaultKey) -> Result<VaultMeta> {
    wrap(passphrase, key)
}

fn wrap(passphrase: &str, key: &VaultKey) -> Result<VaultMeta> {
    if passphrase.trim().is_empty() {
        return Err(AppError::msg("a vault passphrase cannot be empty"));
    }
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    let wrapping = derive(passphrase, &salt)?;

    let wrapped = wrapping.seal_text(&key.to_b64())?;
    let check = wrapping.seal(CHECK_PLAINTEXT)?;
    Ok(VaultMeta {
        version: 1,
        salt: base64(&salt),
        wrapped: base64(&wrapped),
        check: base64(&check),
    })
}

/// Opens the vault with `passphrase`.
///
/// A wrong passphrase is detected by failing to recover [`CHECK_PLAINTEXT`], which
/// is the only honest way to tell "wrong passphrase" from "corrupt database".
pub fn unlock(passphrase: &str, meta: &VaultMeta) -> Result<VaultKey> {
    if meta.version != 1 {
        return Err(AppError::msg(format!(
            "this vault was made by a newer CopyPaste (format {})",
            meta.version
        )));
    }
    let salt = unbase64(&meta.salt)?;
    let wrapping = derive(passphrase, &salt)?;
    let check = unbase64(&meta.check)?;
    let recovered = wrapping
        .open(&check)
        .map_err(|_| AppError::msg("that passphrase does not open the vault"))?;
    if recovered != CHECK_PLAINTEXT {
        return Err(AppError::msg("that passphrase does not open the vault"));
    }
    VaultKey::from_b64(&wrapping.open_text(&unbase64(&meta.wrapped)?)?)
}

/// Stretches a passphrase into a wrapping key with Argon2id.
fn derive(passphrase: &str, salt: &[u8]) -> Result<VaultKey> {
    let params = Params::new(MEMORY_KIB, ITERATIONS, PARALLELISM, Some(DATA_KEY_LEN))
        .map_err(|e| AppError::msg(format!("could not configure the key derivation: {e}")))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = Zeroizing::new([0u8; DATA_KEY_LEN]);
    argon
        .hash_password_into(passphrase.as_bytes(), salt, out.as_mut())
        .map_err(|e| AppError::msg(format!("could not derive the vault key: {e}")))?;
    Ok(VaultKey(out))
}

fn base64(bytes: &[u8]) -> String {
    // The vault's own encoding, so no extra dependency is needed for it: standard
    // alphabet, padded, which every other tool reads back the same way.
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn unbase64(text: &str) -> Result<Vec<u8>> {
    fn value(byte: u8) -> Result<u32> {
        Ok(match byte {
            b'A'..=b'Z' => u32::from(byte - b'A'),
            b'a'..=b'z' => u32::from(byte - b'a') + 26,
            b'0'..=b'9' => u32::from(byte - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return Err(AppError::msg("the vault metadata is not valid base64")),
        })
    }

    let trimmed = text.trim_end_matches('=');
    let mut out = Vec::with_capacity(trimmed.len() / 4 * 3);
    for chunk in trimmed.as_bytes().chunks(4) {
        if chunk.len() < 2 {
            return Err(AppError::msg("the vault metadata is not valid base64"));
        }
        let mut n = 0u32;
        for (index, byte) in chunk.iter().enumerate() {
            n |= value(*byte)? << (18 - 6 * index);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unlocking with the right passphrase returns a key that reads what the
    /// original wrote — the one guarantee everything else rests on.
    #[test]
    fn the_right_passphrase_reopens_what_the_vault_wrote() {
        let (meta, key) = create("correct horse battery staple").unwrap();
        let sealed = key.seal_text("sk_live_hello").unwrap();

        let reopened = unlock("correct horse battery staple", &meta).unwrap();
        assert_eq!(reopened.open_text(&sealed).unwrap(), "sk_live_hello");
    }

    #[test]
    fn the_wrong_passphrase_is_refused_with_a_message_that_says_so() {
        let (meta, _) = create("correct horse battery staple").unwrap();
        // Not `expect_err`: the key deliberately has no `Debug`, so that a stray
        // log line can never print it.
        let error = match unlock("wrong", &meta) {
            Ok(_) => panic!("the wrong passphrase has to fail"),
            Err(e) => e,
        };
        assert!(
            error.to_string().contains("does not open the vault"),
            "the message names the cause: {error}"
        );
    }

    /// A fresh passphrase rewraps the *same* data key, so existing rows keep
    /// opening without being touched.
    #[test]
    fn changing_the_passphrase_keeps_the_same_data_key() {
        let (_, key) = create("first passphrase").unwrap();
        let sealed = key.seal_text("a secret written under the old phrase").unwrap();

        let rewrapped = rewrap("second passphrase", &key).unwrap();
        assert!(
            unlock("first passphrase", &rewrapped).is_err(),
            "the old phrase no longer opens it"
        );
        let reopened = unlock("second passphrase", &rewrapped).unwrap();
        assert_eq!(
            reopened.open_text(&sealed).unwrap(),
            "a secret written under the old phrase"
        );
    }

    /// Every seal uses a fresh nonce, so two identical secrets do not look alike —
    /// otherwise the ciphertext itself would leak that they are equal.
    #[test]
    fn the_same_value_seals_differently_each_time() {
        let (_, key) = create("passphrase").unwrap();
        let first = key.seal_text("same").unwrap();
        let second = key.seal_text("same").unwrap();
        assert_ne!(first, second);
        assert_eq!(key.open_text(&first).unwrap(), "same");
        assert_eq!(key.open_text(&second).unwrap(), "same");
    }

    /// A flipped bit is refused rather than decrypted into garbage.
    #[test]
    fn a_tampered_value_does_not_open() {
        let (_, key) = create("passphrase").unwrap();
        let mut sealed = key.seal_text("payload").unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 0x01;
        assert!(key.open_text(&sealed).is_err());

        let mut truncated = sealed.clone();
        truncated.truncate(NONCE_LEN);
        assert!(key.open_text(&truncated).is_err(), "a bare nonce is not a value");
    }

    #[test]
    fn an_empty_passphrase_is_refused() {
        assert!(create("   ").is_err());
    }

    #[test]
    fn the_meta_survives_a_round_trip_through_json() {
        // It is stored as JSON in `meta`, so it has to come back byte-identical.
        let (meta, _) = create("passphrase").unwrap();
        let json = serde_json::to_string(&meta).unwrap();
        let parsed: VaultMeta = serde_json::from_str(&json).unwrap();
        assert!(unlock("passphrase", &parsed).is_ok());
    }

    #[test]
    fn base64_matches_the_standard_alphabet() {
        // Cross-checked against a known encoding, because this is hand-rolled.
        for (raw, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(raw.as_bytes()), expected, "encoding {raw:?}");
            assert_eq!(unbase64(expected).unwrap(), raw.as_bytes(), "decoding {expected:?}");
        }
    }

    #[test]
    fn a_vault_from_a_future_version_is_refused_rather_than_misread() {
        let (mut meta, _) = create("passphrase").unwrap();
        meta.version = 99;
        assert!(unlock("passphrase", &meta).is_err());
    }
}
