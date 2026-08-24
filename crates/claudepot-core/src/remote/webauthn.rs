//! Passkey verification — ES256 only, no attestation.
//!
//! Implemented here rather than imported. `webauthn-rs` is the correct
//! general answer and pulls native `openssl-sys`, which on Windows needs
//! a system OpenSSL or a vendored build with perl and nasm; that would
//! burden every contributor's build and put the Windows compile gate at
//! risk for a cryptographic generality this surface does not use.
//!
//! The scope is kept narrow so it stays tractable:
//!
//! - **ES256 (P-256 / SHA-256) only.** What Face ID, Touch ID and
//!   Android platform authenticators all produce. An `alg` other than
//!   -7 is refused rather than approximated.
//! - **No attestation verification.** Correct for a self-hosted relying
//!   party that issues credentials to devices it has *already*
//!   authenticated with a password: attestation answers "what kind of
//!   authenticator is this", which matters to an enterprise deciding
//!   whether to trust unknown hardware and not at all here. `none` is
//!   what the client is asked for and anything else is ignored, never
//!   trusted.
//!
//! ## The checks, enumerated
//!
//! Everything security-critical is in [`verify_registration`] and
//! [`verify_assertion`], and both are short on purpose — a list you can
//! hold in your head is the only compensation for not importing a
//! reviewed implementation.
//!
//! Registration: type is `webauthn.create`; challenge matches the one
//! we issued; origin matches exactly; the RP ID hash matches; the
//! user-present flag is set; attested credential data is present.
//!
//! Assertion: type is `webauthn.get`; challenge matches; origin
//! matches; RP ID hash matches; user-present is set; the signature
//! verifies over `authenticatorData || SHA256(clientDataJSON)`; and the
//! counter has not gone backwards.
//!
//! ## Why the counter check is not optional
//!
//! A signature counter that decreases means the credential has been
//! cloned — the one thing a hardware-backed passkey is supposed to make
//! impossible. Apple's platform authenticators report a constant zero,
//! which is explicitly allowed, so "both zero" is accepted and "went
//! down" is refused.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// COSE algorithm identifier for ES256.
const ALG_ES256: i64 = -7;
/// COSE key type: elliptic curve, two coordinates. Label 1.
const COSE_KTY_EC2: i64 = 2;
/// COSE curve: P-256. Label -1.
const COSE_CRV_P256: i64 = 1;

/// User Present. Bit 0 of the authenticator data flags.
const FLAG_UP: u8 = 0x01;
/// User Verified. Bit 2 — the authenticator checked a biometric or a
/// PIN, not merely that someone touched it.
///
/// Both ceremonies ask for `userVerification: "required"`, but that is
/// a request made *to the authenticator by the client*. Nothing about
/// it binds a client we did not write, so the only place the policy can
/// actually be enforced is here, against the bit the authenticator
/// signed over. WebAuthn L2 §7.1 step 15 and §7.2 step 17 say exactly
/// this. Without the check, "unlock with Face ID" degrades silently to
/// "touch the key", which is the whole difference the feature sells.
const FLAG_UV: u8 = 0x04;
/// Attested credential data included. Bit 6.
const FLAG_AT: u8 = 0x40;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WebauthnError {
    #[error("client data is not valid JSON")]
    ClientDataMalformed,
    #[error("expected ceremony type {expected:?}, got {got:?}")]
    WrongCeremony { expected: &'static str, got: String },
    #[error("challenge does not match the one issued")]
    ChallengeMismatch,
    #[error("origin {got:?} is not {expected:?}")]
    OriginMismatch { got: String, expected: String },
    #[error("authenticator data is truncated")]
    AuthDataTruncated,
    #[error("the RP ID hash does not match this relying party")]
    RpIdMismatch,
    #[error("the user-present flag is not set")]
    UserNotPresent,
    #[error("the authenticator did not verify the user (no biometric or PIN)")]
    UserNotVerified,
    #[error("no attested credential data in the registration")]
    NoAttestedCredential,
    #[error("attestation object is not valid CBOR")]
    AttestationMalformed,
    #[error("public key is not ES256 (alg {0}), which is the only algorithm supported here")]
    UnsupportedAlgorithm(i64),
    #[error("public key is malformed")]
    BadPublicKey,
    #[error("signature is malformed")]
    BadSignature,
    #[error("signature does not verify")]
    SignatureInvalid,
    #[error("signature counter went backwards ({got} < {stored}) — the credential may be cloned")]
    CounterRegressed { got: u32, stored: u32 },
    #[error("base64url decode failed")]
    BadBase64,
}

/// A registered credential, as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credential {
    /// base64url, as the browser reports it.
    pub id: String,
    /// SEC1 uncompressed P-256 point, base64url. A public key: safe at
    /// rest, unlike every other secret in this module tree.
    pub public_key: String,
    pub sign_count: u32,
}

#[derive(Debug, Deserialize)]
struct ClientData {
    #[serde(rename = "type")]
    ceremony: String,
    challenge: String,
    origin: String,
}

fn parse_client_data(
    json: &[u8],
    expected_type: &'static str,
    challenge: &[u8],
    origin: &str,
) -> Result<(), WebauthnError> {
    let cd: ClientData =
        serde_json::from_slice(json).map_err(|_| WebauthnError::ClientDataMalformed)?;
    if cd.ceremony != expected_type {
        return Err(WebauthnError::WrongCeremony {
            expected: expected_type,
            got: cd.ceremony,
        });
    }
    let got = URL_SAFE_NO_PAD
        .decode(cd.challenge.as_bytes())
        .map_err(|_| WebauthnError::BadBase64)?;
    // Constant time is not required — the challenge is ours and public
    // once issued — but a length check first avoids a panic-shaped
    // comparison on hostile input.
    if got.len() != challenge.len() || got != challenge {
        return Err(WebauthnError::ChallengeMismatch);
    }
    if cd.origin != origin {
        return Err(WebauthnError::OriginMismatch {
            got: cd.origin,
            expected: origin.to_string(),
        });
    }
    Ok(())
}

struct AuthData<'a> {
    flags: u8,
    sign_count: u32,
    attested: Option<&'a [u8]>,
}

fn parse_auth_data<'a>(data: &'a [u8], rp_id: &str) -> Result<AuthData<'a>, WebauthnError> {
    // rpIdHash(32) || flags(1) || signCount(4) || [attested...]
    if data.len() < 37 {
        return Err(WebauthnError::AuthDataTruncated);
    }
    let expected = Sha256::digest(rp_id.as_bytes());
    if data[..32] != expected[..] {
        return Err(WebauthnError::RpIdMismatch);
    }
    let flags = data[32];
    let sign_count = u32::from_be_bytes([data[33], data[34], data[35], data[36]]);
    let attested = (flags & FLAG_AT != 0).then(|| &data[37..]);
    Ok(AuthData {
        flags,
        sign_count,
        attested,
    })
}

/// Pull the credential id and COSE public key out of attested data.
fn parse_attested(attested: &[u8]) -> Result<(Vec<u8>, Vec<u8>), WebauthnError> {
    // aaguid(16) || credentialIdLength(2) || credentialId || COSEKey
    if attested.len() < 18 {
        return Err(WebauthnError::AuthDataTruncated);
    }
    let id_len = u16::from_be_bytes([attested[16], attested[17]]) as usize;
    let start: usize = 18;
    let end = start
        .checked_add(id_len)
        .ok_or(WebauthnError::AuthDataTruncated)?;
    if attested.len() < end {
        return Err(WebauthnError::AuthDataTruncated);
    }
    Ok((attested[start..end].to_vec(), attested[end..].to_vec()))
}

/// COSE_Key → SEC1 uncompressed point, refusing anything but ES256.
fn cose_to_sec1(cose: &[u8]) -> Result<Vec<u8>, WebauthnError> {
    let value: ciborium::Value =
        ciborium::from_reader(cose).map_err(|_| WebauthnError::BadPublicKey)?;
    let map = value.as_map().ok_or(WebauthnError::BadPublicKey)?;

    let (mut kty, mut alg, mut crv) = (None, None, None);
    let (mut x, mut y) = (None, None);
    for (k, v) in map {
        let Some(k) = k.as_integer().and_then(|i| i64::try_from(i).ok()) else {
            continue;
        };
        match k {
            1 => kty = v.as_integer().and_then(|i| i64::try_from(i).ok()),
            3 => alg = v.as_integer().and_then(|i| i64::try_from(i).ok()),
            -1 => crv = v.as_integer().and_then(|i| i64::try_from(i).ok()),
            -2 => x = v.as_bytes().cloned(),
            -3 => y = v.as_bytes().cloned(),
            _ => {}
        }
    }

    match alg {
        Some(ALG_ES256) => {}
        Some(other) => return Err(WebauthnError::UnsupportedAlgorithm(other)),
        None => return Err(WebauthnError::BadPublicKey),
    }
    // `alg` alone does not say what the bytes ARE. Without `kty` and
    // `crv` an OKP/Ed25519 key labelled `-7` is accepted and its 32-byte
    // fields are reinterpreted as a P-256 affine point — the parser
    // would be trusting a self-declared label over the actual key type.
    // RFC 8152 makes both mandatory for an EC2 key; require them.
    if kty != Some(COSE_KTY_EC2) || crv != Some(COSE_CRV_P256) {
        return Err(WebauthnError::BadPublicKey);
    }
    let (x, y) = (
        x.ok_or(WebauthnError::BadPublicKey)?,
        y.ok_or(WebauthnError::BadPublicKey)?,
    );
    if x.len() != 32 || y.len() != 32 {
        return Err(WebauthnError::BadPublicKey);
    }

    let mut sec1 = Vec::with_capacity(65);
    sec1.push(0x04); // uncompressed
    sec1.extend_from_slice(&x);
    sec1.extend_from_slice(&y);
    Ok(sec1)
}

/// Verify a registration and return the credential to store.
pub fn verify_registration(
    client_data_json: &[u8],
    attestation_object: &[u8],
    challenge: &[u8],
    origin: &str,
    rp_id: &str,
) -> Result<Credential, WebauthnError> {
    parse_client_data(client_data_json, "webauthn.create", challenge, origin)?;

    let att: ciborium::Value = ciborium::from_reader(attestation_object)
        .map_err(|_| WebauthnError::AttestationMalformed)?;
    let map = att.as_map().ok_or(WebauthnError::AttestationMalformed)?;
    let auth_data = map
        .iter()
        .find(|(k, _)| k.as_text() == Some("authData"))
        .and_then(|(_, v)| v.as_bytes())
        .ok_or(WebauthnError::AttestationMalformed)?;

    // `fmt` and `attStmt` are deliberately not read. See the module
    // docs: attestation answers a question this relying party does not
    // ask, and half-verifying it would be worse than not verifying it.

    let parsed = parse_auth_data(auth_data, rp_id)?;
    if parsed.flags & FLAG_UP == 0 {
        return Err(WebauthnError::UserNotPresent);
    }
    if parsed.flags & FLAG_UV == 0 {
        return Err(WebauthnError::UserNotVerified);
    }
    let attested = parsed.attested.ok_or(WebauthnError::NoAttestedCredential)?;
    let (cred_id, cose) = parse_attested(attested)?;
    let sec1 = cose_to_sec1(&cose)?;

    Ok(Credential {
        id: URL_SAFE_NO_PAD.encode(cred_id),
        public_key: URL_SAFE_NO_PAD.encode(sec1),
        sign_count: parsed.sign_count,
    })
}

/// Verify an assertion. Returns the new signature counter to store.
pub fn verify_assertion(
    credential: &Credential,
    client_data_json: &[u8],
    authenticator_data: &[u8],
    signature: &[u8],
    challenge: &[u8],
    origin: &str,
    rp_id: &str,
) -> Result<u32, WebauthnError> {
    parse_client_data(client_data_json, "webauthn.get", challenge, origin)?;
    let parsed = parse_auth_data(authenticator_data, rp_id)?;
    if parsed.flags & FLAG_UP == 0 {
        return Err(WebauthnError::UserNotPresent);
    }
    if parsed.flags & FLAG_UV == 0 {
        return Err(WebauthnError::UserNotVerified);
    }

    let sec1 = URL_SAFE_NO_PAD
        .decode(credential.public_key.as_bytes())
        .map_err(|_| WebauthnError::BadPublicKey)?;
    let key = VerifyingKey::from_sec1_bytes(&sec1).map_err(|_| WebauthnError::BadPublicKey)?;

    // The signed message is authenticatorData || SHA-256(clientDataJSON).
    let mut signed = Vec::with_capacity(authenticator_data.len() + 32);
    signed.extend_from_slice(authenticator_data);
    signed.extend_from_slice(&Sha256::digest(client_data_json));

    // WebAuthn signatures are ASN.1 DER, not fixed-width.
    let sig = Signature::from_der(signature).map_err(|_| WebauthnError::BadSignature)?;
    key.verify(&signed, &sig)
        .map_err(|_| WebauthnError::SignatureInvalid)?;

    // A counter that went backwards means the credential was cloned —
    // the one thing hardware backing is meant to prevent. Apple's
    // platform authenticators report a constant zero, which the spec
    // permits, so equal-at-zero is fine and *decreasing* is not.
    if parsed.sign_count != 0 || credential.sign_count != 0 {
        if parsed.sign_count <= credential.sign_count {
            return Err(WebauthnError::CounterRegressed {
                got: parsed.sign_count,
                stored: credential.sign_count,
            });
        }
    }
    Ok(parsed.sign_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::signature::Signer;
    use p256::ecdsa::SigningKey;

    const RP: &str = "appliance.example.internal";
    const ORIGIN: &str = "https://appliance.example.internal:8420";

    fn client_data(ceremony: &str, challenge: &[u8], origin: &str) -> Vec<u8> {
        format!(
            r#"{{"type":"{ceremony}","challenge":"{}","origin":"{origin}","crossOrigin":false}}"#,
            URL_SAFE_NO_PAD.encode(challenge)
        )
        .into_bytes()
    }

    fn auth_data(rp_id: &str, flags: u8, count: u32, attested: Option<&[u8]>) -> Vec<u8> {
        let mut v = Sha256::digest(rp_id.as_bytes()).to_vec();
        v.push(flags);
        v.extend_from_slice(&count.to_be_bytes());
        if let Some(a) = attested {
            v.extend_from_slice(a);
        }
        v
    }

    /// aaguid || credIdLen || credId || COSE(ES256 pubkey)
    fn attested_blob(key: &VerifyingKey, cred_id: &[u8]) -> Vec<u8> {
        let point = key.to_encoded_point(false);
        let mut cose = Vec::new();
        ciborium::into_writer(
            &ciborium::Value::Map(vec![
                (
                    ciborium::Value::Integer(1.into()),
                    ciborium::Value::Integer(2.into()),
                ),
                (
                    ciborium::Value::Integer(3.into()),
                    ciborium::Value::Integer((-7).into()),
                ),
                (
                    ciborium::Value::Integer((-1).into()),
                    ciborium::Value::Integer(1.into()),
                ),
                (
                    ciborium::Value::Integer((-2).into()),
                    ciborium::Value::Bytes(point.x().unwrap().to_vec()),
                ),
                (
                    ciborium::Value::Integer((-3).into()),
                    ciborium::Value::Bytes(point.y().unwrap().to_vec()),
                ),
            ]),
            &mut cose,
        )
        .unwrap();

        let mut v = vec![0u8; 16];
        v.extend_from_slice(&(cred_id.len() as u16).to_be_bytes());
        v.extend_from_slice(cred_id);
        v.extend_from_slice(&cose);
        v
    }

    fn attestation_object(auth: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        ciborium::into_writer(
            &ciborium::Value::Map(vec![
                (
                    ciborium::Value::Text("fmt".into()),
                    ciborium::Value::Text("none".into()),
                ),
                (
                    ciborium::Value::Text("attStmt".into()),
                    ciborium::Value::Map(vec![]),
                ),
                (
                    ciborium::Value::Text("authData".into()),
                    ciborium::Value::Bytes(auth.to_vec()),
                ),
            ]),
            &mut out,
        )
        .unwrap();
        out
    }

    fn register(challenge: &[u8]) -> (SigningKey, Credential) {
        let sk = key(1);
        let vk = *sk.verifying_key();
        let attested = attested_blob(&vk, b"cred-1");
        let auth = auth_data(RP, FLAG_UP | FLAG_UV | FLAG_AT, 0, Some(&attested));
        let att = attestation_object(&auth);
        let cred = verify_registration(
            &client_data("webauthn.create", challenge, ORIGIN),
            &att,
            challenge,
            ORIGIN,
            RP,
        )
        .expect("registration should verify");
        (sk, cred)
    }

    /// Two DISTINCT deterministic keys.
    ///
    /// The first version of this handed `SigningKey::random` a fake RNG
    /// that filled bytes from a fixed pattern, so every call returned
    /// the *same* key — and `another_keys_signature_does_not_verify`
    /// was quietly comparing a key with itself. A test helper that
    /// silently collapses its inputs makes every test above it weaker
    /// than it reads.
    ///
    /// Fixed scalars instead: deterministic, reproducible, and
    /// unmistakably different.
    fn key(n: u8) -> SigningKey {
        let mut scalar = [0u8; 32];
        scalar[31] = n;
        scalar[0] = 0x0f; // keep it well inside the curve order
        SigningKey::from_slice(&scalar).expect("valid P-256 scalar")
    }

    fn assert_signed(sk: &SigningKey, auth: &[u8], client: &[u8]) -> Vec<u8> {
        let mut signed = auth.to_vec();
        signed.extend_from_slice(&Sha256::digest(client));
        let sig: Signature = sk.sign(&signed);
        sig.to_der().as_bytes().to_vec()
    }

    #[test]
    fn a_registration_yields_a_storable_credential() {
        let (_sk, cred) = register(b"challenge-one-two-three-four-abc");
        assert_eq!(URL_SAFE_NO_PAD.decode(&cred.id).unwrap(), b"cred-1");
        // 0x04 || x(32) || y(32)
        assert_eq!(URL_SAFE_NO_PAD.decode(&cred.public_key).unwrap().len(), 65);
        assert_eq!(cred.sign_count, 0);
    }

    #[test]
    fn the_stored_credential_is_a_public_key_only() {
        let (_sk, cred) = register(b"challenge-one-two-three-four-abc");
        let json = serde_json::to_string(&cred).unwrap();
        assert!(json.contains("public_key"));
        assert!(!json.to_lowercase().contains("private"));
        // First byte 0x04 = uncompressed public point, not a scalar.
        assert_eq!(URL_SAFE_NO_PAD.decode(&cred.public_key).unwrap()[0], 0x04);
    }

    #[test]
    fn a_good_assertion_verifies_and_returns_the_counter() {
        let ch = b"challenge-one-two-three-four-abc";
        let (sk, cred) = register(ch);
        let ch2 = b"second-challenge-aaaaaaaaaaaaaaa";
        let auth = auth_data(RP, FLAG_UP | FLAG_UV, 1, None);
        let client = client_data("webauthn.get", ch2, ORIGIN);
        let sig = assert_signed(&sk, &auth, &client);
        assert_eq!(
            verify_assertion(&cred, &client, &auth, &sig, ch2, ORIGIN, RP).unwrap(),
            1
        );
    }

    #[test]
    fn a_tampered_signature_is_refused() {
        let ch = b"challenge-one-two-three-four-abc";
        let (sk, cred) = register(ch);
        let auth = auth_data(RP, FLAG_UP | FLAG_UV, 1, None);
        let client = client_data("webauthn.get", ch, ORIGIN);
        let mut sig = assert_signed(&sk, &auth, &client);
        let last = sig.len() - 1;
        sig[last] ^= 0xff;
        let got = verify_assertion(&cred, &client, &auth, &sig, ch, ORIGIN, RP);
        assert!(
            matches!(
                got,
                Err(WebauthnError::SignatureInvalid) | Err(WebauthnError::BadSignature)
            ),
            "got {got:?}"
        );
    }

    #[test]
    fn another_keys_signature_does_not_verify() {
        let ch = b"challenge-one-two-three-four-abc";
        let (_sk, cred) = register(ch);
        let other = key(2);
        let auth = auth_data(RP, FLAG_UP | FLAG_UV, 1, None);
        let client = client_data("webauthn.get", ch, ORIGIN);
        let sig = assert_signed(&other, &auth, &client);
        assert_eq!(
            verify_assertion(&cred, &client, &auth, &sig, ch, ORIGIN, RP),
            Err(WebauthnError::SignatureInvalid)
        );
    }

    #[test]
    fn a_replayed_challenge_is_refused() {
        // The heart of it: a captured assertion must not re-authenticate
        // against a freshly issued challenge.
        let ch = b"challenge-one-two-three-four-abc";
        let (sk, cred) = register(ch);
        let auth = auth_data(RP, FLAG_UP | FLAG_UV, 1, None);
        let client = client_data("webauthn.get", ch, ORIGIN);
        let sig = assert_signed(&sk, &auth, &client);
        let fresh = b"a-different-challenge-zzzzzzzzzz";
        assert_eq!(
            verify_assertion(&cred, &client, &auth, &sig, fresh, ORIGIN, RP),
            Err(WebauthnError::ChallengeMismatch)
        );
    }

    #[test]
    fn a_foreign_origin_is_refused() {
        let ch = b"challenge-one-two-three-four-abc";
        let (sk, cred) = register(ch);
        let auth = auth_data(RP, FLAG_UP | FLAG_UV, 1, None);
        let client = client_data("webauthn.get", ch, "https://evil.example");
        let sig = assert_signed(&sk, &auth, &client);
        assert!(matches!(
            verify_assertion(&cred, &client, &auth, &sig, ch, ORIGIN, RP),
            Err(WebauthnError::OriginMismatch { .. })
        ));
    }

    #[test]
    fn a_different_rp_id_is_refused() {
        // Guards against a credential registered to one appliance being
        // presented to another.
        let ch = b"challenge-one-two-three-four-abc";
        let (sk, cred) = register(ch);
        let auth = auth_data("someone-else.internal", FLAG_UP | FLAG_UV, 1, None);
        let client = client_data("webauthn.get", ch, ORIGIN);
        let sig = assert_signed(&sk, &auth, &client);
        assert_eq!(
            verify_assertion(&cred, &client, &auth, &sig, ch, ORIGIN, RP),
            Err(WebauthnError::RpIdMismatch)
        );
    }

    #[test]
    fn the_wrong_ceremony_type_is_refused() {
        // A registration response replayed as a login, and vice versa.
        let ch = b"challenge-one-two-three-four-abc";
        let (sk, cred) = register(ch);
        let auth = auth_data(RP, FLAG_UP | FLAG_UV, 1, None);
        let client = client_data("webauthn.create", ch, ORIGIN);
        let sig = assert_signed(&sk, &auth, &client);
        assert!(matches!(
            verify_assertion(&cred, &client, &auth, &sig, ch, ORIGIN, RP),
            Err(WebauthnError::WrongCeremony { .. })
        ));
    }

    #[test]
    fn a_missing_user_present_flag_is_refused() {
        let ch = b"challenge-one-two-three-four-abc";
        let (sk, cred) = register(ch);
        let auth = auth_data(RP, FLAG_UV, 1, None);
        let client = client_data("webauthn.get", ch, ORIGIN);
        let sig = assert_signed(&sk, &auth, &client);
        assert_eq!(
            verify_assertion(&cred, &client, &auth, &sig, ch, ORIGIN, RP),
            Err(WebauthnError::UserNotPresent)
        );
    }

    #[test]
    fn a_regressed_counter_is_refused_as_a_clone() {
        let ch = b"challenge-one-two-three-four-abc";
        let (sk, _) = register(ch);
        let mut cred = register(ch).1;
        cred.sign_count = 10;
        // Re-register with the first key so the stored public key matches.
        let (sk2, mut cred2) = (sk, cred);
        cred2.public_key = {
            let vk = sk2.verifying_key().to_encoded_point(false);
            URL_SAFE_NO_PAD.encode(vk.as_bytes())
        };
        let auth = auth_data(RP, FLAG_UP | FLAG_UV, 5, None);
        let client = client_data("webauthn.get", ch, ORIGIN);
        let sig = assert_signed(&sk2, &auth, &client);
        assert_eq!(
            verify_assertion(&cred2, &client, &auth, &sig, ch, ORIGIN, RP),
            Err(WebauthnError::CounterRegressed { got: 5, stored: 10 })
        );
    }

    #[test]
    fn a_constant_zero_counter_is_accepted() {
        // Apple's platform authenticators always report zero, and the
        // spec permits it. Treating "not greater" as a clone here would
        // lock out every iPhone.
        let ch = b"challenge-one-two-three-four-abc";
        let (sk, cred) = register(ch);
        assert_eq!(cred.sign_count, 0);
        let auth = auth_data(RP, FLAG_UP | FLAG_UV, 0, None);
        let client = client_data("webauthn.get", ch, ORIGIN);
        let sig = assert_signed(&sk, &auth, &client);
        assert_eq!(
            verify_assertion(&cred, &client, &auth, &sig, ch, ORIGIN, RP).unwrap(),
            0
        );
    }

    #[test]
    fn a_non_es256_key_is_refused_rather_than_approximated() {
        let mut cose = Vec::new();
        ciborium::into_writer(
            &ciborium::Value::Map(vec![
                // alg = -257 (RS256)
                (
                    ciborium::Value::Integer(3.into()),
                    ciborium::Value::Integer((-257).into()),
                ),
                (
                    ciborium::Value::Integer((-2).into()),
                    ciborium::Value::Bytes(vec![0u8; 32]),
                ),
                (
                    ciborium::Value::Integer((-3).into()),
                    ciborium::Value::Bytes(vec![0u8; 32]),
                ),
            ]),
            &mut cose,
        )
        .unwrap();
        assert_eq!(
            cose_to_sec1(&cose),
            Err(WebauthnError::UnsupportedAlgorithm(-257))
        );
    }

    #[test]
    fn truncated_input_does_not_panic() {
        // Everything here is attacker-supplied; a panic is a denial of
        // service on a surface reachable from the network.
        for n in 0..64 {
            let short = vec![0u8; n];
            let _ = parse_auth_data(&short, RP);
            let _ = parse_attested(&short);
            let _ = cose_to_sec1(&short);
            let _ = verify_registration(b"{}", &short, b"c", ORIGIN, RP);
        }
    }

    #[test]
    fn a_credential_id_length_beyond_the_buffer_is_refused() {
        // The classic length-prefix overrun.
        let mut attested = vec![0u8; 16];
        attested.extend_from_slice(&u16::MAX.to_be_bytes());
        attested.extend_from_slice(b"short");
        assert_eq!(
            parse_attested(&attested),
            Err(WebauthnError::AuthDataTruncated)
        );
    }

    #[test]
    fn an_assertion_without_user_verification_is_refused() {
        // `userVerification: "required"` is a request the client makes
        // to the authenticator. A client we did not write can ignore it
        // and return a UP-only assertion, so the server has to check the
        // signed bit. Without this the Face ID login silently accepts a
        // mere touch.
        let ch = b"challenge-one-two-three-four-abc";
        let (sk, cred) = register(ch);
        let auth = auth_data(RP, FLAG_UP, 1, None);
        let client = client_data("webauthn.get", ch, ORIGIN);
        let sig = assert_signed(&sk, &auth, &client);
        assert_eq!(
            verify_assertion(&cred, &client, &auth, &sig, ch, ORIGIN, RP),
            Err(WebauthnError::UserNotVerified)
        );
    }

    #[test]
    fn a_registration_without_user_verification_is_refused() {
        // Same policy at enrolment: a credential registered without user
        // verification cannot later produce a verified assertion, so
        // accepting it here would enrol a passkey that can never satisfy
        // the login check.
        let ch = b"challenge-one-two-three-four-abc";
        let sk = key(3);
        let attested = attested_blob(sk.verifying_key(), b"cred-id-bytes");
        let auth = auth_data(RP, FLAG_UP | FLAG_AT, 0, Some(&attested));
        let client = client_data("webauthn.create", ch, ORIGIN);
        let att = attestation_object(&auth);
        assert_eq!(
            verify_registration(&client, &att, ch, ORIGIN, RP),
            Err(WebauthnError::UserNotVerified)
        );
    }

    #[test]
    fn a_cose_key_that_is_not_ec2_p256_is_refused() {
        // `alg: -7` is a label, not proof of what the bytes are. An
        // OKP/Ed25519 key wearing that label has 32-byte fields too, and
        // would be reinterpreted as a P-256 point.
        let sk = key(3);
        let point = sk.verifying_key().to_encoded_point(false);
        let build = |kty: i64, crv: i64| {
            let mut cose = Vec::new();
            ciborium::into_writer(
                &ciborium::Value::Map(vec![
                    (
                        ciborium::Value::Integer(1.into()),
                        ciborium::Value::Integer(kty.into()),
                    ),
                    (
                        ciborium::Value::Integer(3.into()),
                        ciborium::Value::Integer((-7).into()),
                    ),
                    (
                        ciborium::Value::Integer((-1).into()),
                        ciborium::Value::Integer(crv.into()),
                    ),
                    (
                        ciborium::Value::Integer((-2).into()),
                        ciborium::Value::Bytes(point.x().unwrap().to_vec()),
                    ),
                    (
                        ciborium::Value::Integer((-3).into()),
                        ciborium::Value::Bytes(point.y().unwrap().to_vec()),
                    ),
                ]),
                &mut cose,
            )
            .unwrap();
            cose
        };
        // OKP instead of EC2.
        assert_eq!(cose_to_sec1(&build(1, 1)), Err(WebauthnError::BadPublicKey));
        // EC2 but the wrong curve (P-384).
        assert_eq!(cose_to_sec1(&build(2, 2)), Err(WebauthnError::BadPublicKey));
        // The real thing still parses.
        assert!(cose_to_sec1(&build(2, 1)).is_ok());
    }

    #[test]
    fn a_cose_key_missing_kty_or_crv_is_refused() {
        let sk = key(3);
        let point = sk.verifying_key().to_encoded_point(false);
        let mut cose = Vec::new();
        ciborium::into_writer(
            &ciborium::Value::Map(vec![
                (
                    ciborium::Value::Integer(3.into()),
                    ciborium::Value::Integer((-7).into()),
                ),
                (
                    ciborium::Value::Integer((-2).into()),
                    ciborium::Value::Bytes(point.x().unwrap().to_vec()),
                ),
                (
                    ciborium::Value::Integer((-3).into()),
                    ciborium::Value::Bytes(point.y().unwrap().to_vec()),
                ),
            ]),
            &mut cose,
        )
        .unwrap();
        assert_eq!(cose_to_sec1(&cose), Err(WebauthnError::BadPublicKey));
    }
}
