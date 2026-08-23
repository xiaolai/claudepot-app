// WebAuthn plumbing: base64url on the wire, ArrayBuffers in the API.
//
// Two facts shape everything here, both learned the expensive way:
//
//   - **The RP ID must be a hostname.** WebAuthn requires a valid
//     domain, and an IP-address origin has none — so `https://100.x.x.x`
//     cannot register a passkey however capable the phone is.
//   - **`isUserVerifyingPlatformAuthenticatorAvailable()` answers the
//     wrong question.** It reports whether the DEVICE has Face ID, and
//     returns `true` on exactly the origin that cannot use it. Availability
//     is therefore `device && origin`, never the flag alone.

export function b64urlToBytes(s) {
  const pad = s.length % 4 === 0 ? '' : '='.repeat(4 - (s.length % 4));
  const bin = window.atob(s.replace(/-/g, '+').replace(/_/g, '/') + pad);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i += 1) out[i] = bin.charCodeAt(i);
  return out;
}

export function bytesToB64url(buf) {
  const bytes = new Uint8Array(buf);
  let bin = '';
  for (let i = 0; i < bytes.length; i += 1) bin += String.fromCharCode(bytes[i]);
  return window.btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

/** True when this origin's host is a name a browser will accept as an RP ID. */
export function originCanHostPasskeys() {
  const h = window.location.hostname;
  if (!h) return false;
  // Bracketed IPv6, or anything that parses as a dotted quad.
  if (h.startsWith('[') || /^\d{1,3}(\.\d{1,3}){3}$/.test(h)) return false;
  return true;
}

/** `{ device, origin }` — both must hold before a passkey is offered. */
export async function passkeySupport() {
  const origin = originCanHostPasskeys();
  let device = false;
  try {
    device =
      typeof window.PublicKeyCredential !== 'undefined' &&
      (await window.PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable());
  } catch {
    device = false;
  }
  return { device, origin, usable: device && origin };
}

/** Why a passkey is unavailable, in words the user can act on. */
export function passkeyBlocker(support) {
  if (!support.origin) {
    return `Reached by IP address. A passkey needs a hostname — open this panel by its name (for example ${window.location.hostname ? 'the machine’s MagicDNS or .local name' : 'its hostname'}) and try again.`;
  }
  if (!support.device) return 'This device has no platform authenticator (Face ID / Touch ID).';
  return null;
}

export async function createCredential(options) {
  const cred = await navigator.credentials.create({
    publicKey: {
      ...options,
      challenge: b64urlToBytes(options.challenge),
      user: { ...options.user, id: b64urlToBytes(options.user.id) },
      excludeCredentials: (options.excludeCredentials || []).map((c) => ({
        ...c,
        id: b64urlToBytes(c.id),
      })),
    },
  });
  if (!cred) throw new Error('cancelled');
  return {
    id: cred.id,
    client_data_json: bytesToB64url(cred.response.clientDataJSON),
    attestation_object: bytesToB64url(cred.response.attestationObject),
  };
}

export async function getAssertion(options) {
  const cred = await navigator.credentials.get({
    publicKey: {
      ...options,
      challenge: b64urlToBytes(options.challenge),
      allowCredentials: (options.allowCredentials || []).map((c) => ({
        ...c,
        id: b64urlToBytes(c.id),
      })),
    },
  });
  if (!cred) throw new Error('cancelled');
  return {
    id: cred.id,
    client_data_json: bytesToB64url(cred.response.clientDataJSON),
    authenticator_data: bytesToB64url(cred.response.authenticatorData),
    signature: bytesToB64url(cred.response.signature),
  };
}
