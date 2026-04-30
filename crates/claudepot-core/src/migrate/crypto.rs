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
//! ## Signing (`minisign`) — sign the manifest digest
//!
//! Optional. When `--sign KEYFILE` is set, the **canonical bytes of
//! `manifest.json`** (body + the self-sha trailer that already lands
//! in the tar) are signed with the supplied minisign secret key. The
//! signature is written next to the final artifact as
//! `<bundle>.manifest.minisig` — same neighbor for plaintext bundles
//! (`*.tar.zst`) and encrypted ones (`*.tar.zst.age`).
//!
//! Why sign the manifest, not the bundle bytes:
//!   1. The manifest carries `FileInventoryEntry { path, size, sha256 }`
//!      for every payload file, plus a sha256 of itself in the trailer.
//!      Signing the manifest therefore commits to every byte of every
//!      payload by transitivity — without forcing the verifier to read
//!      the whole archive before deciding whether to trust it.
//!   2. The signature is independent of encryption: the same `.minisig`
//!      is valid for both the plaintext and encrypted forms of the same
//!      logical bundle. The previous shape (sign bundle bytes) wrote
//!      the sidecar at the plaintext path and then deleted the
//!      plaintext during encryption, leaving an orphaned sidecar that
//!      could never be verified — so encrypted+signed never produced
//!      a verifiable artifact.
//!   3. Aligns with `dev-docs/project-migrate-spec.md` §3.3.
//!
//! On import, after the manifest is read out of the bundle (decrypting
//! first if needed), `verify_manifest_signature` is called against the
//! sidecar and the user-supplied public key (`--verify-key PUBFILE`).
//! Per-file integrity is then enforced by `verify_extracted_dir`'s pass
//! over `integrity.sha256` — which the manifest digest already covers.

use crate::migrate::error::MigrateError;
use age::secrecy::SecretString;
use std::fs;
use std::path::{Path, PathBuf};

/// Suffix appended to the bundle filename when encryption is enabled.
pub const ENCRYPTED_SUFFIX: &str = ".age";

/// Suffix for the manifest-digest minisign signature file. Distinct
/// from the legacy `.minisig` (which signed bundle bytes) so old and
/// new artifacts can't be confused at the filesystem level.
pub const MANIFEST_SIGNATURE_SUFFIX: &str = ".manifest.minisig";

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

    // Atomic write: encrypt to a unique sibling temp, fsync, then
    // rename into place. Without this, an encryption failure (or an
    // interruption mid-stream) leaves a truncated `.age` file at the
    // final path, and any pre-existing `.age` at that path is gone.
    // The `.tmp.<pid>` name is unique per concurrent caller and lives
    // in the same dir as the final path, so the rename stays atomic.
    let parent = out_path.parent().ok_or_else(|| {
        MigrateError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("encrypt target has no parent dir: {}", out_path.display()),
        ))
    })?;
    let tmp_name = format!(
        ".{}.tmp.{}",
        out_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("bundle.age"),
        std::process::id()
    );
    let tmp_path = parent.join(tmp_name);

    // Cleanup guard: if anything below errors out, remove the temp
    // so we don't litter `.tmp.<pid>` files on failure.
    struct TempGuard(PathBuf, bool);
    impl Drop for TempGuard {
        fn drop(&mut self) {
            if !self.1 {
                let _ = fs::remove_file(&self.0);
            }
        }
    }
    let mut guard = TempGuard(tmp_path.clone(), false);

    let plaintext_file = fs::File::open(plaintext_bundle).map_err(MigrateError::from)?;
    let mut reader = std::io::BufReader::new(plaintext_file);
    let out_file = fs::File::create(&tmp_path).map_err(MigrateError::from)?;
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
    drop(out_file);

    // Atomic rename now: any pre-existing `.age` at `out_path` is
    // overwritten in one syscall, never truncated-then-overwritten.
    fs::rename(&tmp_path, &out_path).map_err(MigrateError::from)?;
    guard.1 = true; // success — don't clean up the temp (it's been renamed)
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
    let _ = tmp
        .into_temp_path()
        .keep()
        .map_err(|e| MigrateError::Io(std::io::Error::other(format!("keep tempfile: {e}"))))?;
    Ok(tmp_path)
}

/// Compute the sidecar path `<artifact>.manifest.minisig`. Public so
/// the export path can describe the produced artifacts in its receipt
/// without re-deriving the suffix.
pub fn manifest_signature_path_for(artifact: &Path) -> PathBuf {
    let mut s = artifact.as_os_str().to_os_string();
    s.push(MANIFEST_SIGNATURE_SUFFIX);
    PathBuf::from(s)
}

/// Sign the canonical `manifest.json` bytes (body + self-sha trailer,
/// exactly as written into the bundle's tar) with the supplied minisign
/// secret key, and write the signature to
/// `<artifact>.manifest.minisig`. `artifact` is the final shipping path
/// — the encrypted `.age` when encryption ran, the plaintext `.tar.zst`
/// otherwise — so the sidecar always sits next to whichever file the
/// user distributes.
///
/// `password` is the secret-key passphrase. Pass `Some("")` (or `None`,
/// which we coerce to empty) for unprotected keys; an actual interactive
/// prompt would be unusable in CI / scripted contexts.
///
/// Returns the path to the written `.manifest.minisig` file.
pub fn sign_manifest_digest(
    artifact: &Path,
    manifest_bytes: &[u8],
    secret_key_file: &Path,
    password: Option<String>,
) -> Result<PathBuf, MigrateError> {
    let password = Some(password.unwrap_or_default());
    let sk = minisign::SecretKey::from_file(secret_key_file, password)
        .map_err(|e| MigrateError::IntegrityViolation(format!("minisign secret key load: {e}")))?;
    // minisign::sign takes any Read; an in-memory cursor over the
    // manifest bytes keeps the call site uniform with the streaming
    // shape minisign expects.
    let signature = minisign::sign(None, &sk, std::io::Cursor::new(manifest_bytes), None, None)
        .map_err(|e| MigrateError::Serialize(format!("minisign sign: {e}")))?;
    let out_path = manifest_signature_path_for(artifact);
    fs::write(&out_path, signature.into_string()).map_err(MigrateError::from)?;
    Ok(out_path)
}

/// Verify `<artifact>.manifest.minisig` against the canonical
/// manifest bytes (which the caller has already extracted from the
/// bundle, decrypting first if needed) and the user-supplied public
/// key. Returns `Ok(())` only on a valid signature; missing sidecar,
/// parse failure, or signature mismatch becomes `IntegrityViolation`.
///
/// The artifact path is used to derive the sidecar location only;
/// the function never reads the artifact bytes, which is the entire
/// point of this protocol shape — verification is `O(manifest_size)`,
/// independent of bundle size and unchanged by encryption.
pub fn verify_manifest_signature(
    artifact: &Path,
    manifest_bytes: &[u8],
    public_key_file: &Path,
) -> Result<(), MigrateError> {
    let sig_path = manifest_signature_path_for(artifact);
    if !sig_path.exists() {
        return Err(MigrateError::IntegrityViolation(format!(
            "manifest signature missing: {}",
            sig_path.display()
        )));
    }
    let pk_bytes = fs::read_to_string(public_key_file).map_err(MigrateError::from)?;
    let pk = minisign_verify::PublicKey::decode(&pk_bytes)
        .map_err(|e| MigrateError::IntegrityViolation(format!("minisign public key parse: {e}")))?;
    let sig_bytes = fs::read_to_string(&sig_path).map_err(MigrateError::from)?;
    let sig = minisign_verify::Signature::decode(&sig_bytes)
        .map_err(|e| MigrateError::IntegrityViolation(format!("minisign signature parse: {e}")))?;
    pk.verify(manifest_bytes, &sig, false)
        .map_err(|e| MigrateError::IntegrityViolation(format!("signature verify: {e}")))?;
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
        let decrypted = decrypt_bundle_with_passphrase(&enc, &pwd, &decrypt_tmpdir).unwrap();
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
        let enc = encrypt_bundle_with_passphrase(&plaintext_path, &passphrase("pw")).unwrap();
        // Now we simulate a SECOND, unrelated plaintext at the same
        // sibling path that should NOT be overwritten by a decrypt
        // pass.
        fs::write(&plaintext_path, b"unrelated-pre-existing").unwrap();
        let stage = tmp.path().join("stage");
        let _decrypted = decrypt_bundle_with_passphrase(&enc, &passphrase("pw"), &stage).unwrap();
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
        let enc = encrypt_bundle_with_passphrase(&plaintext_path, &passphrase("right")).unwrap();
        let err =
            decrypt_bundle_with_passphrase(&enc, &passphrase("wrong"), &tmp.path().join("stage"))
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
            decrypt_bundle_with_passphrase(&p, &passphrase("anything"), &tmp.path().join("stage"))
                .unwrap_err();
        assert!(matches!(err, MigrateError::IntegrityViolation(_)));
    }

    /// Helper: produce an unprotected minisign keypair on disk. The
    /// "encrypted" name is misleading — even unprotected keys go
    /// through the empty-password XOR so `SecretKey::from_file(path,
    /// Some(""))` round-trips.
    fn make_keypair(tmp: &Path) -> (PathBuf, PathBuf) {
        use minisign::KeyPair;
        let kp = KeyPair::generate_encrypted_keypair(Some(String::new())).unwrap();
        let sk_path = tmp.join("minisign.key");
        fs::write(&sk_path, kp.sk.to_box(None).unwrap().into_string()).unwrap();
        let pk_path = tmp.join("minisign.pub");
        fs::write(&pk_path, kp.pk.to_box().unwrap().into_string()).unwrap();
        (sk_path, pk_path)
    }

    #[test]
    fn sign_manifest_digest_writes_sidecar_at_artifact() {
        // The signature sidecar is named relative to the *final*
        // artifact (encrypted or not), not the manifest bytes; this
        // test pins that naming so encrypted+signed and plain+signed
        // both put the .manifest.minisig in the predictable place.
        let tmp = tempfile::tempdir().unwrap();
        let (sk_path, _pk_path) = make_keypair(tmp.path());

        let artifact = tmp.path().join("foo.tar.zst.age");
        let manifest_bytes = b"{\"schema_version\":1}\n# manifest-sha256: deadbeef\n";

        let sig_path = sign_manifest_digest(&artifact, manifest_bytes, &sk_path, None).unwrap();
        // The suffix is APPENDED, not replacing the extension — paths
        // like `foo.tar.zst.age` need `foo.tar.zst.age.manifest.minisig`,
        // not `foo.tar.zst.manifest.minisig`. `Path::with_extension`
        // would replace the last component, which is the wrong shape.
        let mut expected_os = artifact.as_os_str().to_os_string();
        expected_os.push(MANIFEST_SIGNATURE_SUFFIX);
        assert_eq!(sig_path, PathBuf::from(expected_os));
        assert!(sig_path.exists());
    }

    #[test]
    fn manifest_sign_verify_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let (sk_path, pk_path) = make_keypair(tmp.path());
        let artifact = tmp.path().join("rt.tar.zst");
        // Artifact contents are irrelevant — this protocol signs
        // manifest bytes, never the artifact. We still create the
        // file so the sidecar has a real neighbor.
        fs::write(&artifact, b"opaque").unwrap();

        let manifest_bytes = b"manifest body + trailer";
        sign_manifest_digest(&artifact, manifest_bytes, &sk_path, None).unwrap();
        verify_manifest_signature(&artifact, manifest_bytes, &pk_path).unwrap();
    }

    #[test]
    fn manifest_verify_rejects_tampered_manifest_bytes() {
        // Core property: signature is over the manifest, so flipping
        // a byte in the manifest must fail verification even though
        // the artifact path and sidecar are untouched.
        let tmp = tempfile::tempdir().unwrap();
        let (sk_path, pk_path) = make_keypair(tmp.path());
        let artifact = tmp.path().join("t.tar.zst");
        fs::write(&artifact, b"opaque").unwrap();

        let original = b"original manifest";
        sign_manifest_digest(&artifact, original, &sk_path, None).unwrap();

        let tampered = b"tampered manifest";
        let err = verify_manifest_signature(&artifact, tampered, &pk_path).unwrap_err();
        assert!(matches!(err, MigrateError::IntegrityViolation(_)));
    }

    #[test]
    fn manifest_verify_rejects_missing_sidecar() {
        // No sidecar present → IntegrityViolation, not Io. The
        // distinction matters because the verify call is the import-
        // path gate; a missing sidecar must fail closed (refuse import)
        // rather than slip through as a transient I/O error.
        let tmp = tempfile::tempdir().unwrap();
        let (_sk_path, pk_path) = make_keypair(tmp.path());
        let artifact = tmp.path().join("never-signed.tar.zst");
        fs::write(&artifact, b"opaque").unwrap();

        let err = verify_manifest_signature(&artifact, b"any", &pk_path).unwrap_err();
        assert!(matches!(err, MigrateError::IntegrityViolation(_)));
    }

    #[test]
    fn manifest_signature_path_appends_suffix() {
        // Belt-and-braces: the suffix is `.manifest.minisig` (not
        // bare `.minisig`) so legacy artifacts can't be confused with
        // the new shape at the filesystem level.
        let p = manifest_signature_path_for(Path::new("a/b/c.tar.zst.age"));
        assert!(p
            .to_string_lossy()
            .ends_with(".tar.zst.age.manifest.minisig"));
    }
}
