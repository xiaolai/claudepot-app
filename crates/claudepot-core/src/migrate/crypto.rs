//! Bundle encryption + signing.
//!
//! See `dev-docs/project-migrate-spec.md` §3.3.
//!
//! ## Encryption (`age`)
//!
//! Default-on for end-user bundles. The wire format is `age`'s
//! ASCII-armored container wrapping the raw `*.claudepot.tar.zst`
//! payload, written to `*.claudepot.tar.zst.age`.
//!
//! Passphrase mode only for v1 — recipient-key mode (X25519) requires
//! key distribution UX that's out of scope. Passphrase derivation is
//! `scrypt(N=2^18, r=8, p=1)` per the `age` defaults; payload symmetric
//! key is ChaCha20-Poly1305.
//!
//! `--no-encrypt` opts out (e.g. for pipelines that already run over
//! encrypted transport).
//!
//! ## Signing (`minisign`)
//!
//! Optional. When `--sign KEYFILE` is set, the manifest sha256 trailer
//! is signed with the supplied minisign secret key, and the signature
//! is written next to the bundle as `<bundle>.minisig`. Required for
//! `--unattended-import`.
//!
//! Verification on the import side reads the sidecar `<bundle>.minisig`
//! and a public key (provided via `--verify-key PUBFILE` or by trust
//! lookup against `~/.claudepot/trust/`).

use crate::migrate::error::MigrateError;
use age::secrecy::SecretString;
use std::fs;
use std::path::{Path, PathBuf};

/// Suffix appended to the bundle filename when encryption is enabled.
pub const ENCRYPTED_SUFFIX: &str = ".age";

/// Suffix for the minisign signature file.
pub const SIGNATURE_SUFFIX: &str = ".minisig";

/// Encrypt a plaintext bundle into a sibling `.age` file using a
/// passphrase. Returns the path to the encrypted output. The plaintext
/// is **not** removed by this function — the caller chooses whether
/// to retain it (debug mode) or delete it (default).
pub fn encrypt_bundle_with_passphrase(
    plaintext_bundle: &Path,
    passphrase: &SecretString,
) -> Result<PathBuf, MigrateError> {
    let plaintext = fs::read(plaintext_bundle).map_err(MigrateError::from)?;
    let encryptor = age::Encryptor::with_user_passphrase(passphrase.clone());

    let mut out = Vec::new();
    let mut writer = encryptor
        .wrap_output(&mut out)
        .map_err(|e| MigrateError::Serialize(format!("age encrypt: {e}")))?;
    use std::io::Write;
    writer
        .write_all(&plaintext)
        .map_err(MigrateError::from)?;
    writer
        .finish()
        .map_err(|e| MigrateError::Serialize(format!("age finish: {e}")))?;

    let mut out_path_os = plaintext_bundle.as_os_str().to_os_string();
    out_path_os.push(ENCRYPTED_SUFFIX);
    let out_path = PathBuf::from(out_path_os);
    fs::write(&out_path, out).map_err(MigrateError::from)?;
    Ok(out_path)
}

/// Decrypt a `.age` bundle to a sibling plaintext file (the same path
/// minus the `.age` suffix). Returns the plaintext path. Wrong
/// passphrase produces `IntegrityViolation` (no partial extraction).
pub fn decrypt_bundle_with_passphrase(
    encrypted_bundle: &Path,
    passphrase: &SecretString,
) -> Result<PathBuf, MigrateError> {
    let encrypted = fs::read(encrypted_bundle).map_err(MigrateError::from)?;
    let decryptor = age::Decryptor::new(&encrypted[..])
        .map_err(|e| MigrateError::IntegrityViolation(format!("age open: {e}")))?;

    // age 0.10's Decryptor is an enum; passphrase mode lives on the
    // Passphrase variant. A non-passphrase bundle (recipient mode)
    // here is a configuration error — reject loudly.
    let mut reader = match decryptor {
        age::Decryptor::Passphrase(d) => d
            .decrypt(passphrase, None)
            .map_err(|e| MigrateError::IntegrityViolation(format!("age decrypt: {e}")))?,
        age::Decryptor::Recipients(_) => {
            return Err(MigrateError::IntegrityViolation(
                "bundle is recipient-encrypted, not passphrase-encrypted".to_string(),
            ));
        }
    };

    let mut plaintext = Vec::new();
    use std::io::Read;
    reader
        .read_to_end(&mut plaintext)
        .map_err(MigrateError::from)?;

    let stem = encrypted_bundle
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| {
            MigrateError::IntegrityViolation(
                "encrypted bundle has no filename".to_string(),
            )
        })?;
    let plaintext_name = stem.strip_suffix(ENCRYPTED_SUFFIX).ok_or_else(|| {
        MigrateError::IntegrityViolation(format!(
            "encrypted bundle filename does not end with {ENCRYPTED_SUFFIX}"
        ))
    })?;
    let parent = encrypted_bundle.parent().unwrap_or_else(|| Path::new("."));
    let out = parent.join(plaintext_name);
    fs::write(&out, plaintext).map_err(MigrateError::from)?;
    Ok(out)
}

/// Sign the bundle (entire file bytes) using a minisign secret key.
/// The signature is over the bundle bytes, giving us a one-shot
/// integrity + authenticity gate that survives transport.
///
/// `password` is the secret-key passphrase. Pass `Some("")` for
/// unprotected keys; passing `None` causes minisign to prompt
/// interactively, which is unusable in CI / scripted contexts. The
/// migrate CLI normalizes its `--sign KEYFILE [--sign-password PASS]`
/// args into the right shape before calling this.
///
/// Returns the path to the `.minisig` file written next to the bundle.
pub fn sign_bundle(
    bundle: &Path,
    secret_key_file: &Path,
    password: Option<String>,
) -> Result<PathBuf, MigrateError> {
    // Default to empty-string (unencrypted-key flow) rather than None
    // (interactive prompt). Callers that want the prompt can pass
    // `password = Some(...)` directly.
    let password = Some(password.unwrap_or_default());
    let sk = minisign::SecretKey::from_file(secret_key_file, password).map_err(|e| {
        MigrateError::IntegrityViolation(format!("minisign secret key load: {e}"))
    })?;
    let bundle_file = fs::File::open(bundle).map_err(MigrateError::from)?;
    let signature = minisign::sign(None, &sk, bundle_file, None, None)
        .map_err(|e| MigrateError::Serialize(format!("minisign sign: {e}")))?;
    let mut out_os = bundle.as_os_str().to_os_string();
    out_os.push(SIGNATURE_SUFFIX);
    let out_path = PathBuf::from(out_os);
    fs::write(&out_path, signature.into_string()).map_err(MigrateError::from)?;
    Ok(out_path)
}

/// Verify the bundle's `.minisig` sidecar against a known public key.
/// Returns `Ok(())` only on a valid signature; mismatch or any I/O
/// error becomes `IntegrityViolation`. Use this on the import path
/// when the user supplies `--verify-key`.
pub fn verify_bundle_signature(
    bundle: &Path,
    public_key_file: &Path,
) -> Result<(), MigrateError> {
    let mut sig_path = bundle.as_os_str().to_os_string();
    sig_path.push(SIGNATURE_SUFFIX);
    let sig_path = PathBuf::from(sig_path);
    if !sig_path.exists() {
        return Err(MigrateError::IntegrityViolation(format!(
            "bundle signature missing: {}",
            sig_path.display()
        )));
    }
    let pk_bytes = fs::read_to_string(public_key_file).map_err(MigrateError::from)?;
    let pk = minisign_verify::PublicKey::decode(&pk_bytes).map_err(|e| {
        MigrateError::IntegrityViolation(format!("minisign public key parse: {e}"))
    })?;
    let sig_bytes = fs::read_to_string(&sig_path).map_err(MigrateError::from)?;
    let sig = minisign_verify::Signature::decode(&sig_bytes).map_err(|e| {
        MigrateError::IntegrityViolation(format!("minisign signature parse: {e}"))
    })?;
    let bundle_bytes = fs::read(bundle).map_err(MigrateError::from)?;
    pk.verify(&bundle_bytes, &sig, false).map_err(|e| {
        MigrateError::IntegrityViolation(format!("signature verify: {e}"))
    })?;
    Ok(())
}

// ---------------------------------------------------------------------
// Backwards-compat helpers — used by the orchestrator's pre-flight
// before the encrypt path landed. The previous "always refuse" stubs
// are retained for callers that want to opt out explicitly without
// supplying a passphrase.
// ---------------------------------------------------------------------

/// No-op pass-through retained for callers that want explicit
/// "I want plaintext" branching. When `encrypt` is true and no
/// passphrase pipeline exists yet, the orchestrator should call
/// `encrypt_bundle_with_passphrase` directly instead of this.
pub fn require_plaintext_only(_encrypt: bool) -> Result<(), MigrateError> {
    Ok(())
}

/// Same shape: optional pre-flight hook. Real signing goes through
/// `sign_bundle`.
pub fn require_unsigned(_keyfile: Option<&str>) -> Result<(), MigrateError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passphrase(s: &str) -> SecretString {
        SecretString::from(s.to_string())
    }

    #[test]
    fn encrypt_decrypt_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let plaintext_path = tmp.path().join("payload.tar.zst");
        let plaintext = b"hello world, this is a fake bundle payload\n";
        fs::write(&plaintext_path, plaintext).unwrap();

        let pwd = passphrase("correct horse battery staple");
        let enc = encrypt_bundle_with_passphrase(&plaintext_path, &pwd).unwrap();
        assert!(enc.exists());
        // age envelope is heavier than plaintext for tiny inputs.
        assert!(fs::metadata(&enc).unwrap().len() > 0);
        // Original plaintext is left in place.
        assert_eq!(fs::read(&plaintext_path).unwrap(), plaintext);

        // Remove plaintext and decrypt to a fresh location.
        fs::remove_file(&plaintext_path).unwrap();
        let decrypted = decrypt_bundle_with_passphrase(&enc, &pwd).unwrap();
        assert_eq!(decrypted, plaintext_path);
        assert_eq!(fs::read(&decrypted).unwrap(), plaintext);
    }

    #[test]
    fn decrypt_with_wrong_passphrase_refuses() {
        let tmp = tempfile::tempdir().unwrap();
        let plaintext_path = tmp.path().join("p.tar.zst");
        fs::write(&plaintext_path, b"x").unwrap();
        let enc = encrypt_bundle_with_passphrase(&plaintext_path, &passphrase("right"))
            .unwrap();
        let err = decrypt_bundle_with_passphrase(&enc, &passphrase("wrong"))
            .unwrap_err();
        match err {
            MigrateError::IntegrityViolation(_) => {}
            other => panic!("expected IntegrityViolation, got {other:?}"),
        }
    }

    #[test]
    fn decrypt_refuses_non_age_input() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("not-encrypted.tar.zst.age");
        fs::write(&p, b"this is not an age file").unwrap();
        let err =
            decrypt_bundle_with_passphrase(&p, &passphrase("anything")).unwrap_err();
        assert!(matches!(err, MigrateError::IntegrityViolation(_)));
    }

    #[test]
    fn require_plaintext_only_is_pass_through() {
        // Backwards compat for callers that still gate on the boolean.
        assert!(require_plaintext_only(true).is_ok());
        assert!(require_plaintext_only(false).is_ok());
    }

    #[test]
    fn sign_and_verify_round_trip() {
        // minisign keys at rest are always "encrypted" — even an
        // unprotected key uses the empty-password XOR so the checksum
        // round-trips. `generate_encrypted_keypair(Some(""))` is the
        // canonical way to produce a key that loads via
        // `SecretKey::from_file(path, Some(""))`.
        use minisign::KeyPair;
        let tmp = tempfile::tempdir().unwrap();

        let kp = KeyPair::generate_encrypted_keypair(Some(String::new())).unwrap();
        let sk_path = tmp.path().join("minisign.key");
        fs::write(&sk_path, kp.sk.to_box(None).unwrap().into_string()).unwrap();
        let pk_path = tmp.path().join("minisign.pub");
        fs::write(&pk_path, kp.pk.to_box().unwrap().into_string()).unwrap();

        let bundle_path = tmp.path().join("b.tar.zst");
        fs::write(&bundle_path, b"bundle bytes").unwrap();
        let sig_path = sign_bundle(&bundle_path, &sk_path, None).unwrap();
        assert!(sig_path.exists());
        verify_bundle_signature(&bundle_path, &pk_path).unwrap();

        // Tamper one byte → verify fails.
        fs::write(&bundle_path, b"tampered bytes").unwrap();
        let err = verify_bundle_signature(&bundle_path, &pk_path).unwrap_err();
        assert!(matches!(err, MigrateError::IntegrityViolation(_)));
    }
}
