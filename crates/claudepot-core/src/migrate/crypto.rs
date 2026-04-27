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
//! Optional. When `--sign KEYFILE` is set, the **entire bundle file
//! bytes** are signed with the supplied minisign secret key, and the
//! signature is written next to the bundle as `<bundle>.minisig`.
//!
//! This is a deliberate deviation from `dev-docs/project-migrate-spec.md`
//! §3.3, which proposes signing only the manifest sha256. Signing the
//! bundle bytes is strictly stronger (covers payload tampering as well
//! as manifest tampering), at the cost of forcing the verifier to
//! read the whole file. The deviation is documented in `bundle.rs`
//! and is the contract this module implements; a future PR can either
//! align both sides if the spec wants the lighter form.
//!
//! Verification on the import side reads the sidecar `<bundle>.minisig`
//! and a public key (provided via `--verify-key PUBFILE`).

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
///
/// Streams plaintext → age encryptor → output file via buffered I/O
/// so we don't buffer the entire bundle in memory (audit Performance
/// finding). The intermediate buffer is bounded by `BufWriter`'s
/// default capacity (8KB).
pub fn encrypt_bundle_with_passphrase(
    plaintext_bundle: &Path,
    passphrase: &SecretString,
) -> Result<PathBuf, MigrateError> {
    let mut out_path_os = plaintext_bundle.as_os_str().to_os_string();
    out_path_os.push(ENCRYPTED_SUFFIX);
    let out_path = PathBuf::from(out_path_os);

    let plaintext_file = fs::File::open(plaintext_bundle).map_err(MigrateError::from)?;
    let mut reader = std::io::BufReader::new(plaintext_file);
    let out_file = fs::File::create(&out_path).map_err(MigrateError::from)?;
    let writer_buf = std::io::BufWriter::new(out_file);

    let encryptor = age::Encryptor::with_user_passphrase(passphrase.clone());
    let mut age_writer = encryptor
        .wrap_output(writer_buf)
        .map_err(|e| MigrateError::Serialize(format!("age encrypt: {e}")))?;
    std::io::copy(&mut reader, &mut age_writer).map_err(MigrateError::from)?;
    let buf_writer = age_writer
        .finish()
        .map_err(|e| MigrateError::Serialize(format!("age finish: {e}")))?;
    let mut out_file = buf_writer
        .into_inner()
        .map_err(|e| MigrateError::from(e.into_error()))?;
    use std::io::Write;
    out_file.flush().map_err(MigrateError::from)?;
    out_file.sync_all().map_err(MigrateError::from)?;
    Ok(out_path)
}

/// Decrypt a `.age` bundle into a unique tempfile under the
/// caller-supplied directory. Returns the plaintext path AND a
/// matching sidecar path (since downstream `BundleReader::open` no
/// longer requires the original sidecar — we hand it the plaintext
/// directly via the unverified path).
///
/// Why a tempfile, not a deterministic sibling: a sibling at
/// `<encrypted>` minus `.age` could collide with an existing plaintext
/// bundle, and the cleanup guard would then delete the user's
/// pre-existing file on import success. Using `NamedTempFile` keeps
/// the path unpredictable and outside the user's working tree.
pub fn decrypt_bundle_with_passphrase(
    encrypted_bundle: &Path,
    passphrase: &SecretString,
    tmpdir: &Path,
) -> Result<PathBuf, MigrateError> {
    // Stream the encrypted bytes through age's decryptor into the
    // tempfile via std::io::copy. The earlier shape buffered the
    // whole encrypted payload AND the whole plaintext in memory
    // (audit Performance finding); now both stages are bounded by
    // the BufReader/BufWriter defaults.
    let enc_file = fs::File::open(encrypted_bundle).map_err(MigrateError::from)?;
    let enc_reader = std::io::BufReader::new(enc_file);
    let decryptor = age::Decryptor::new(enc_reader)
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

    fs::create_dir_all(tmpdir).map_err(MigrateError::from)?;
    // Tempfile inside the caller's staging area. We keep the path
    // inside our tempdir so the cleanup guard can remove it without
    // touching the user's tree.
    let tmp = tempfile::Builder::new()
        .prefix("decrypted-")
        .suffix(".tar.zst")
        .tempfile_in(tmpdir)
        .map_err(MigrateError::from)?;
    let tmp_path = tmp.path().to_path_buf();
    {
        let out_file = tmp.as_file();
        let mut buf_writer = std::io::BufWriter::new(out_file);
        std::io::copy(&mut reader, &mut buf_writer).map_err(MigrateError::from)?;
        use std::io::Write;
        buf_writer.flush().map_err(MigrateError::from)?;
    }
    // Detach so the file outlives this function; cleanup is the
    // caller's responsibility (PlaintextCleanupGuard).
    let _ = tmp.into_temp_path().keep().map_err(|e| {
        MigrateError::Io(std::io::Error::other(format!("keep tempfile: {e}")))
    })?;
    Ok(tmp_path)
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

        // Remove plaintext and decrypt to a fresh location. The
        // decrypt landing tempdir is a separate path we control —
        // keeping the decrypted plaintext outside the user's
        // working tree.
        fs::remove_file(&plaintext_path).unwrap();
        let decrypt_tmpdir = tmp.path().join("staging");
        let decrypted =
            decrypt_bundle_with_passphrase(&enc, &pwd, &decrypt_tmpdir).unwrap();
        assert!(decrypted.starts_with(&decrypt_tmpdir));
        assert_eq!(fs::read(&decrypted).unwrap(), plaintext);
        // The original plaintext sibling path was NOT touched.
        assert!(!plaintext_path.exists());
    }

    #[test]
    fn decrypt_does_not_clobber_existing_plaintext_sibling() {
        // Regression for the audit Sec finding: prior version wrote to
        // a deterministic sibling, which would overwrite (and the
        // cleanup guard would then delete) any pre-existing plaintext.
        let tmp = tempfile::tempdir().unwrap();
        let plaintext_path = tmp.path().join("p.tar.zst");
        fs::write(&plaintext_path, b"original-content").unwrap();
        let enc = encrypt_bundle_with_passphrase(&plaintext_path, &passphrase("pw"))
            .unwrap();
        // Now we simulate a SECOND, unrelated plaintext at the same
        // sibling path that should NOT be overwritten by a decrypt
        // pass.
        fs::write(&plaintext_path, b"unrelated-pre-existing").unwrap();
        let stage = tmp.path().join("stage");
        let _decrypted = decrypt_bundle_with_passphrase(
            &enc,
            &passphrase("pw"),
            &stage,
        )
        .unwrap();
        // The pre-existing sibling is intact.
        assert_eq!(
            fs::read(&plaintext_path).unwrap(),
            b"unrelated-pre-existing"
        );
    }

    #[test]
    fn decrypt_with_wrong_passphrase_refuses() {
        let tmp = tempfile::tempdir().unwrap();
        let plaintext_path = tmp.path().join("p.tar.zst");
        fs::write(&plaintext_path, b"x").unwrap();
        let enc = encrypt_bundle_with_passphrase(&plaintext_path, &passphrase("right"))
            .unwrap();
        let err = decrypt_bundle_with_passphrase(
            &enc,
            &passphrase("wrong"),
            &tmp.path().join("stage"),
        )
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
        let err = decrypt_bundle_with_passphrase(
            &p,
            &passphrase("anything"),
            &tmp.path().join("stage"),
        )
        .unwrap_err();
        assert!(matches!(err, MigrateError::IntegrityViolation(_)));
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
