//! Chromium OSCrypt / Electron safeStorage decryption for Claude Desktop.
//!
//! Desktop stores `oauth:tokenCache` inside `config.json` as a base64
//! string with a three-byte version prefix. The envelope is the same
//! format Chromium's cookie store uses — we reuse the well-documented
//! algorithms here.
//!
//! # macOS algorithm (verified against `reference.md` §II.6)
//!
//! ```text
//! cipher_bytes = base64_decode(cipher_b64)       // "v10" prefix + ct
//! require cipher[..3] == b"v10"
//! ct = cipher[3..]
//! key = PBKDF2-HMAC-SHA1(
//!   password = keychain_secret,     // Claude Safe Storage / Claude Key
//!   salt     = b"saltysalt",
//!   iter     = 1003,
//!   dklen    = 16,                  // AES-128
//! )
//! iv = b" " * 16                    // Chromium's fixed IV
//! plaintext = AES-128-CBC-Decrypt(key, iv, ct) + PKCS#7 unpad
//! ```
//!
//! # Windows algorithm
//!
//! ```text
//! cipher_bytes = base64_decode(cipher_b64)
//! require cipher[..3] == b"v10"
//! nonce = cipher[3..15]            // 12 bytes (GCM spec)
//! tag   = cipher[len-16..]         // 16 bytes
//! ct    = cipher[15..len-16]
//! plaintext = AES-256-GCM-Decrypt(master_key, nonce, ct, tag)
//! ```
//!
//! The `master_key` comes from DPAPI-unprotecting the `encrypted_key`
//! stored in `Local State`; that step lives in the platform
//! implementation, not here.

use base64::Engine;

#[derive(Debug, thiserror::Error)]
pub enum DecryptError {
    #[error("ciphertext format: {0}")]
    BadFormat(String),
    #[error("unsupported version tag: expected v10 or v11, got {0:?}")]
    UnknownVersion([u8; 3]),
    #[error("base64 decode failed: {0}")]
    Base64(String),
    #[error("AES decrypt failed (corrupt ciphertext or wrong key)")]
    Aes,
}

/// Strip and validate the Chromium `v10`/`v11` version prefix.
///
/// The three-byte tag is ASCII `v10` for the modern envelope. `v11`
/// appears in some newer Chromium builds; we accept it for forward
/// compatibility because the envelope layout is the same on macOS.
pub fn strip_version_prefix(cipher: &[u8]) -> Result<&[u8], DecryptError> {
    if cipher.len() < 3 {
        return Err(DecryptError::BadFormat("ciphertext shorter than 3 bytes".into()));
    }
    let tag = [cipher[0], cipher[1], cipher[2]];
    if &tag != b"v10" && &tag != b"v11" {
        return Err(DecryptError::UnknownVersion(tag));
    }
    Ok(&cipher[3..])
}

/// Decode base64 → bytes. Handles both STANDARD and STANDARD_NO_PAD —
/// Electron has historically mixed the two.
pub fn b64_decode(s: &str) -> Result<Vec<u8>, DecryptError> {
    base64::engine::general_purpose::STANDARD
        .decode(s.as_bytes())
        .or_else(|_| {
            base64::engine::general_purpose::STANDARD_NO_PAD
                .decode(s.as_bytes())
        })
        .map_err(|e| DecryptError::Base64(e.to_string()))
}

#[cfg(target_os = "macos")]
pub mod macos {
    //! PBKDF2 key derivation + AES-128-CBC with Chromium's fixed IV.

    use super::*;
    use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
    use hmac::Hmac;
    use sha1::Sha1;

    type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

    const SALT: &[u8] = b"saltysalt";
    const ITERATIONS: u32 = 1003;
    const KEY_LEN: usize = 16; // AES-128
    const IV: [u8; 16] = [b' '; 16];

    /// Derive Chromium's AES key from the keychain password.
    ///
    /// Pulled out so tests can drive known-password/known-ciphertext
    /// fixtures through the exact same derivation the production path
    /// uses.
    pub fn derive_key(password: &[u8]) -> [u8; KEY_LEN] {
        let mut key = [0u8; KEY_LEN];
        pbkdf2::pbkdf2::<Hmac<Sha1>>(password, SALT, ITERATIONS, &mut key)
            .expect("PBKDF2 output length is hard-coded");
        key
    }

    /// Decrypt a base64 `v10…` ciphertext using the Chromium OSCrypt
    /// envelope. `secret` is the value of the `Claude Safe Storage` /
    /// `Claude Key` keychain password.
    pub fn decrypt(cipher_b64: &str, secret: &[u8]) -> Result<Vec<u8>, DecryptError> {
        let raw = b64_decode(cipher_b64)?;
        let ct = strip_version_prefix(&raw)?;
        let key = derive_key(secret);
        Aes128CbcDec::new_from_slices(&key, &IV)
            .map_err(|_| DecryptError::Aes)?
            .decrypt_padded_vec_mut::<Pkcs7>(ct)
            .map_err(|_| DecryptError::Aes)
    }
}

#[cfg(target_os = "windows")]
pub mod windows {
    //! AES-256-GCM with a DPAPI-unwrapped master key.

    use super::*;
    use aes_gcm::{aead::Aead, Aes256Gcm, Key, KeyInit, Nonce};

    const NONCE_LEN: usize = 12;
    const TAG_LEN: usize = 16;

    /// Decrypt a base64 `v10…` ciphertext using the Windows Electron
    /// safeStorage envelope. `master_key` must be 32 bytes (256-bit
    /// AES key) — produced by DPAPI-unprotecting the `encrypted_key`
    /// field of `Local State`.
    pub fn decrypt(cipher_b64: &str, master_key: &[u8]) -> Result<Vec<u8>, DecryptError> {
        if master_key.len() != 32 {
            return Err(DecryptError::BadFormat(format!(
                "master_key must be 32 bytes, got {}",
                master_key.len()
            )));
        }
        let raw = b64_decode(cipher_b64)?;
        let body = strip_version_prefix(&raw)?;
        if body.len() < NONCE_LEN + TAG_LEN {
            return Err(DecryptError::BadFormat(
                "ciphertext too short for GCM envelope".into(),
            ));
        }
        let nonce = Nonce::from_slice(&body[..NONCE_LEN]);
        let ct_and_tag = &body[NONCE_LEN..];
        let key = Key::<Aes256Gcm>::from_slice(master_key);
        let cipher = Aes256Gcm::new(key);
        cipher
            .decrypt(nonce, ct_and_tag)
            .map_err(|_| DecryptError::Aes)
    }
}

// ---------------------------------------------------------------------------
// Tests — round-trip fixtures generated from the same algorithm. Not recorded
// real ciphertext (that requires a sacrificial Desktop install + keychain
// access); instead we encrypt synthetic plaintext with a known secret and
// assert the decrypter recovers it.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_version_v10_ok() {
        let s = strip_version_prefix(b"v10hello").unwrap();
        assert_eq!(s, b"hello");
    }

    #[test]
    fn test_strip_version_v11_ok() {
        // v11 is accepted for forward compatibility with newer Chromium.
        let s = strip_version_prefix(b"v11hello").unwrap();
        assert_eq!(s, b"hello");
    }

    #[test]
    fn test_strip_version_rejects_unknown() {
        let err = strip_version_prefix(b"v99hello").unwrap_err();
        assert!(matches!(err, DecryptError::UnknownVersion(_)));
    }

    #[test]
    fn test_strip_version_rejects_short() {
        let err = strip_version_prefix(b"x").unwrap_err();
        assert!(matches!(err, DecryptError::BadFormat(_)));
    }

    #[test]
    fn test_b64_decode_standard() {
        assert_eq!(b64_decode("aGVsbG8=").unwrap(), b"hello");
    }

    #[test]
    fn test_b64_decode_no_padding() {
        // Missing padding — STANDARD_NO_PAD fallback path.
        assert_eq!(b64_decode("aGVsbG8").unwrap(), b"hello");
    }

    #[test]
    fn test_b64_decode_garbage() {
        let err = b64_decode("@@@").unwrap_err();
        assert!(matches!(err, DecryptError::Base64(_)));
    }

    #[cfg(target_os = "macos")]
    mod macos_tests {
        use super::super::*;
        use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
        type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

        fn encrypt_like_chromium(plaintext: &[u8], secret: &[u8]) -> String {
            let key = super::super::macos::derive_key(secret);
            let iv = [b' '; 16];
            let ct = Aes128CbcEnc::new_from_slices(&key, &iv)
                .unwrap()
                .encrypt_padded_vec_mut::<Pkcs7>(plaintext);
            let mut envelope = Vec::with_capacity(3 + ct.len());
            envelope.extend_from_slice(b"v10");
            envelope.extend_from_slice(&ct);
            base64::engine::general_purpose::STANDARD.encode(envelope)
        }

        #[test]
        fn test_round_trip_small() {
            let secret = b"Claude Safe Storage test password";
            let plaintext = b"hello world";
            let envelope = encrypt_like_chromium(plaintext, secret);
            let decrypted = macos::decrypt(&envelope, secret).unwrap();
            assert_eq!(decrypted, plaintext);
        }

        #[test]
        fn test_round_trip_json_shaped() {
            // Matches the expected oauth:tokenCache plaintext shape.
            let secret = b"test-keychain-password-12345";
            let plaintext = br#"{"access_token":"sk-ant-oat01-AAA","refresh_token":"sk-ant-ort01-BBB","expires_at":"2026-04-23T14:00:00Z"}"#;
            let envelope = encrypt_like_chromium(plaintext, secret);
            let decrypted = macos::decrypt(&envelope, secret).unwrap();
            assert_eq!(decrypted, plaintext);
        }

        #[test]
        fn test_wrong_password_fails_cleanly() {
            let secret = b"right password";
            let wrong = b"wrong password";
            let envelope = encrypt_like_chromium(b"plaintext", secret);
            let err = macos::decrypt(&envelope, wrong).unwrap_err();
            assert!(matches!(err, DecryptError::Aes));
        }

        #[test]
        fn test_corrupt_ciphertext_fails_cleanly() {
            let secret = b"password";
            let mut envelope = encrypt_like_chromium(b"plaintext", secret);
            // Flip a byte somewhere in the ciphertext part.
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(envelope.as_bytes())
                .unwrap();
            let mut corrupt = bytes.clone();
            let last = corrupt.len() - 1;
            corrupt[last] ^= 0xff;
            envelope = base64::engine::general_purpose::STANDARD.encode(&corrupt);
            let err = macos::decrypt(&envelope, secret).unwrap_err();
            assert!(matches!(err, DecryptError::Aes));
        }

        #[test]
        fn test_derive_key_deterministic() {
            let k1 = macos::derive_key(b"same");
            let k2 = macos::derive_key(b"same");
            let k3 = macos::derive_key(b"other");
            assert_eq!(k1, k2);
            assert_ne!(k1, k3);
        }
    }

    #[cfg(target_os = "windows")]
    mod windows_tests {
        use super::super::*;
        use aes_gcm::{aead::Aead, Aes256Gcm, Key, KeyInit, Nonce};

        fn encrypt_like_electron_win(plaintext: &[u8], master_key: &[u8]) -> String {
            let key = Key::<Aes256Gcm>::from_slice(master_key);
            let cipher = Aes256Gcm::new(key);
            let nonce_bytes = [0u8; 12]; // deterministic for test
            let nonce = Nonce::from_slice(&nonce_bytes);
            let ct = cipher.encrypt(nonce, plaintext).unwrap();
            let mut envelope = Vec::with_capacity(3 + 12 + ct.len());
            envelope.extend_from_slice(b"v10");
            envelope.extend_from_slice(&nonce_bytes);
            envelope.extend_from_slice(&ct);
            base64::engine::general_purpose::STANDARD.encode(envelope)
        }

        #[test]
        fn test_round_trip_windows() {
            let master = [0x42u8; 32];
            let plaintext = b"hello windows";
            let envelope = encrypt_like_electron_win(plaintext, &master);
            assert_eq!(windows::decrypt(&envelope, &master).unwrap(), plaintext);
        }

        #[test]
        fn test_wrong_key_fails_cleanly() {
            let right = [0x42u8; 32];
            let wrong = [0x43u8; 32];
            let envelope = encrypt_like_electron_win(b"pt", &right);
            assert!(matches!(
                windows::decrypt(&envelope, &wrong).unwrap_err(),
                DecryptError::Aes
            ));
        }

        #[test]
        fn test_bad_master_key_length() {
            let bad = [0u8; 20];
            assert!(matches!(
                windows::decrypt("v10XXXX", &bad).unwrap_err(),
                DecryptError::BadFormat(_)
            ));
        }
    }
}
