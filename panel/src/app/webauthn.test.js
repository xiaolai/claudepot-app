// WebAuthn plumbing.
//
// Auth-critical and entirely pure, which is the combination that most
// deserves tests and most easily goes without them. Two things are
// checked: base64url survives a round trip in both directions (a wrong
// alphabet or a dropped pad silently corrupts a signature), and the
// origin gate refuses every shape of IP literal.
//
// The origin gate is the one worth being pedantic about. It is the only
// thing standing between the user and a registration ceremony that
// cannot possibly succeed — the browser reports a platform
// authenticator as available on exactly the origin that has no relying
// party.
import { test } from 'node:test';
import assert from 'node:assert/strict';

import { b64urlToBytes, bytesToB64url, originCanHostPasskeys, passkeyBlocker } from './webauthn.js';

/** Point the module's `window.location.hostname` at a value. */
function withHost(hostname, fn) {
  const prev = globalThis.window;
  globalThis.window = {
    location: { hostname },
    atob: (s) => Buffer.from(s, 'base64').toString('binary'),
    btoa: (s) => Buffer.from(s, 'binary').toString('base64'),
  };
  try {
    return fn();
  } finally {
    globalThis.window = prev;
  }
}

test('base64url survives a round trip at every padding length', () => {
  withHost('host.local', () => {
    // 0, 1 and 2 bytes of padding — the three cases a naive
    // implementation gets wrong one of.
    for (const len of [1, 2, 3, 32, 65, 100]) {
      const bytes = new Uint8Array(len);
      for (let i = 0; i < len; i += 1) bytes[i] = (i * 37 + 11) % 256;
      const encoded = bytesToB64url(bytes);
      assert.ok(!encoded.includes('='), `padding leaked at length ${len}: ${encoded}`);
      assert.ok(!encoded.includes('+') && !encoded.includes('/'), `wrong alphabet: ${encoded}`);
      assert.deepEqual(Array.from(b64urlToBytes(encoded)), Array.from(bytes), `round trip failed at ${len}`);
    }
  });
});

test('base64url decodes the url-safe alphabet, not the standard one', () => {
  withHost('host.local', () => {
    // Bytes chosen to force `-` and `_` into the encoding.
    const bytes = new Uint8Array([0xfb, 0xff, 0xbf]);
    const encoded = bytesToB64url(bytes);
    assert.equal(encoded, '-_-_');
    assert.deepEqual(Array.from(b64urlToBytes(encoded)), Array.from(bytes));
  });
});

test('an IP-literal origin cannot host a passkey', () => {
  // The trap: `isUserVerifyingPlatformAuthenticatorAvailable()` answers
  // "does this DEVICE have Face ID" and returns true here.
  // Documentation/reserved ranges only. A real address from someone's
  // network would test nothing extra and would outlive the test.
  for (const host of ['100.64.0.1', '192.0.2.10', '127.0.0.1', '198.51.100.7', '[2001:db8::1]']) {
    withHost(host, () => {
      assert.equal(originCanHostPasskeys(), false, host);
    });
  }
});

test('a named origin can host a passkey', () => {
  for (const host of ['laptop.local', 'laptop.tailnet-example.ts.net', 'localhost', 'laptop']) {
    withHost(host, () => {
      assert.equal(originCanHostPasskeys(), true, host);
    });
  }
});

test('an empty hostname is refused rather than assumed', () => {
  withHost('', () => {
    assert.equal(originCanHostPasskeys(), false);
  });
});

test('the blocker names the origin problem before the device problem', () => {
  // Both can be false at once, and only one of them is something the
  // user can act on by moving. Telling someone their phone has no Face
  // ID when the real problem is the URL sends them to the wrong place.
  withHost('100.64.1.2', () => {
    const msg = passkeyBlocker({ origin: false, device: false, usable: false });
    assert.match(msg, /IP address/);
  });
  withHost('laptop.local', () => {
    const msg = passkeyBlocker({ origin: true, device: false, usable: false });
    assert.match(msg, /platform authenticator/);
  });
  withHost('laptop.local', () => {
    assert.equal(passkeyBlocker({ origin: true, device: true, usable: true }), null);
  });
});
