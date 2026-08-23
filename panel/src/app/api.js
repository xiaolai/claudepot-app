// The fetch layer. One place that knows about the bearer token, the
// error shape, and the idempotency contract.
//
// Every request is same-origin and relative. The panel is served by the
// appliance it talks to, so there is no base URL to configure and no
// host to get wrong — and the CSP the server sends (`connect-src
// 'self'`) would refuse anything else anyway.

const TOKEN_KEY = 'claudepot.token';

export function readToken() {
  try {
    return window.localStorage.getItem(TOKEN_KEY) || null;
  } catch {
    // Private browsing on iOS throws rather than returning null.
    return null;
  }
}

export function writeToken(token) {
  try {
    if (token) window.localStorage.setItem(TOKEN_KEY, token);
    else window.localStorage.removeItem(TOKEN_KEY);
  } catch {
    /* Non-fatal: the session lives for this page load only. */
  }
}

/** Thrown for every non-2xx. `code` is the server's stable error slug. */
export class ApiError extends Error {
  constructor(status, code, retryAfterSecs) {
    super(code);
    this.name = 'ApiError';
    this.status = status;
    this.code = code;
    this.retryAfterSecs = retryAfterSecs ?? null;
  }
}

/** Thrown when the request never reached the host at all. */
export class OfflineError extends Error {
  constructor(cause) {
    super('offline');
    this.name = 'OfflineError';
    this.cause = cause;
  }
}

async function request(method, path, { body, idempotencyKey, signal } = {}) {
  const headers = {};
  const token = readToken();
  if (token) headers.Authorization = `Bearer ${token}`;
  if (body !== undefined) headers['Content-Type'] = 'application/json';
  if (idempotencyKey) headers['Idempotency-Key'] = idempotencyKey;

  let res;
  try {
    res = await fetch(path, {
      method,
      headers,
      signal,
      body: body === undefined ? undefined : JSON.stringify(body),
      // The token is a bearer header, not a cookie. Sending credentials
      // would add a CSRF surface the design does not need.
      credentials: 'omit',
      cache: 'no-store',
    });
  } catch (e) {
    if (e?.name === 'AbortError') throw e;
    throw new OfflineError(e);
  }

  if (res.status === 204) return null;

  let payload = null;
  const text = await res.text();
  if (text) {
    try {
      payload = JSON.parse(text);
    } catch {
      payload = null;
    }
  }

  if (!res.ok) {
    throw new ApiError(res.status, payload?.error || `http_${res.status}`, payload?.retry_after_secs);
  }
  return payload;
}

/**
 * A fresh idempotency key per user intent.
 *
 * The server refuses a mutation without one, because this is the
 * endpoint where a mobile retry does the work twice. Generated at the
 * call site rather than inside `request` so a retry of the *same*
 * intent reuses the key and a new intent does not.
 */
export function newIdempotencyKey() {
  if (window.crypto?.randomUUID) return window.crypto.randomUUID();
  const b = new Uint8Array(16);
  window.crypto.getRandomValues(b);
  return Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');
}

export const api = {
  health: () => request('GET', '/api/health'),
  me: () => request('GET', '/api/me'),

  login: (password, totp, deviceLabel) =>
    request('POST', '/api/login', {
      body: { password, totp: totp || null, device_label: deviceLabel || null },
    }),

  sessions: (signal) => request('GET', '/api/sessions', { signal }),

  // Three ways to ask, because a phone opening a 900-event transcript
  // wants the END, and "page forward from zero" would show it the wrong
  // end of the story.
  //   tail   — the last `limit` events (opening a thread)
  //   since  — the server's `next_cursor`, handed back verbatim
  //   before — the `limit` events ending just under an index (scroll up)
  //
  // `since` is a COUNT of consumed events, not the index of the last
  // one. Do not decrement it: an earlier version sent `next_cursor - 1`
  // against an exclusive comparison and lost exactly one event per poll.
  transcriptTail: (id, { limit = 60, signal } = {}) =>
    request('GET', `/api/sessions/${encodeURIComponent(id)}/transcript?tail=1&limit=${limit}`, {
      signal,
    }),

  transcriptSince: (id, since, { limit = 60, signal } = {}) =>
    request('GET', `/api/sessions/${encodeURIComponent(id)}/transcript?since=${since}&limit=${limit}`, {
      signal,
    }),

  transcriptBefore: (id, before, { limit = 60, signal } = {}) =>
    request('GET', `/api/sessions/${encodeURIComponent(id)}/transcript?before=${before}&limit=${limit}`, {
      signal,
    }),

  sendPrompt: (id, text, key) =>
    request('POST', `/api/sessions/${encodeURIComponent(id)}/prompt`, {
      body: { text },
      idempotencyKey: key,
    }),

  // `throughCount` is a transcript page's `total` — a count of events
  // consumed, not the index of the last one. The two differ by one and
  // the server's field is named for the count.
  markRead: (id, throughCount, key) =>
    request('POST', `/api/sessions/${encodeURIComponent(id)}/read`, {
      body: { through_count: throughCount },
      idempotencyKey: key,
    }),

  projects: (signal) => request('GET', '/api/projects', { signal }),
  accounts: (signal) => request('GET', '/api/accounts', { signal }),

  inbound: () => request('GET', '/api/inbound'),
  inboundRevoke: (key) => request('POST', '/api/inbound/revoke', { idempotencyKey: key }),

  passkeyRegisterBegin: () => request('POST', '/api/passkey/register/begin', { body: {} }),
  passkeyRegisterFinish: (body, key) =>
    request('POST', '/api/passkey/register/finish', { body, idempotencyKey: key }),
  passkeyLoginBegin: () => request('POST', '/api/passkey/login/begin', { body: {} }),
  passkeyLoginFinish: (body) => request('POST', '/api/passkey/login/finish', { body }),
};

/**
 * What went wrong with a mutation, in words that name the next move.
 *
 * Here rather than in a screen: the one-tap reply on the sessions list
 * and the thread composer hit the same endpoint, and two copies of this
 * would explain one failure two different ways.
 */
export function explainSend(e) {
  if (e instanceof OfflineError) return 'Cannot reach this Mac — nothing was sent.';
  if (e instanceof ApiError) {
    switch (e.code) {
      case 'session_gone':
        return 'That session is no longer running.';
      case 'session_not_addressable':
        return 'That session has no message inbox.';
      case 'idempotency_key_in_flight':
        return 'Still sending the previous one — try again in a moment.';
      case 'send_failed':
        return 'Claude Code did not accept it. It may have just exited.';
      default:
        return 'Could not hand that off.';
    }
  }
  return 'Could not hand that off.';
}
