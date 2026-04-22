/**
 * Defense-in-depth UI-side secret redactor. The Rust backend's
 * `session_export::redact_secrets` already runs over the
 * highest-risk fields (`first_user_prompt` at codec.rs::upsert_row,
 * snippets at session_search::make_hit). This module is the second
 * layer: any frontend code that renders user-controlled strings
 * routes them through `redactSecrets()` first, so a single backend
 * regression can't surface a raw `sk-ant-*` token in the DOM.
 *
 * Algorithm is byte-for-byte parity with the Rust redactor:
 *   - find every `sk-ant-` followed by a token-character run
 *     (alnum / `-` / `_`)
 *   - if the run is preceded immediately by `sk-ant-` AND followed
 *     immediately by `*`, it's already redacted — skip past the
 *     full mask so we don't re-wrap into `sk-ant-******<last4>`
 *   - otherwise replace with `sk-ant-***<last4>` (or just
 *     `sk-ant-***` when the token is too short to safely expose
 *     a suffix, mirroring the Rust threshold of 12 chars)
 *
 * The function is idempotent — calling it on already-redacted text
 * is a no-op.
 */

const NEEDLE = "sk-ant-";

export function redactSecrets(text: string): string {
  if (!text || !text.includes(NEEDLE)) return text;
  let out = "";
  let cursor = 0;
  while (cursor < text.length) {
    const start = text.indexOf(NEEDLE, cursor);
    if (start === -1) {
      out += text.slice(cursor);
      break;
    }
    const tokenEnd = scanTokenEnd(text, start);
    // Idempotency guard: if the next char is `*`, the token is
    // already in the mask form. Skip past the full `sk-ant-***<last4>`
    // run so re-redaction is a no-op.
    if (tokenEnd < text.length && text[tokenEnd] === "*") {
      const maskEnd = skipExistingMask(text, tokenEnd);
      out += text.slice(cursor, maskEnd);
      cursor = maskEnd;
      continue;
    }
    out += text.slice(cursor, start);
    out += mask(text.slice(start, tokenEnd));
    cursor = tokenEnd;
  }
  return out;
}

/**
 * Idempotent on `null` / `undefined`: returns the input unchanged so
 * call sites can do `<span title={maybeRedact(prompt)}>` without an
 * extra `??` guard.
 */
export function maybeRedact<T extends string | null | undefined>(
  text: T,
): T {
  if (text == null) return text;
  return redactSecrets(text) as T;
}

function scanTokenEnd(text: string, start: number): number {
  let i = start;
  while (i < text.length && isTokenChar(text.charCodeAt(i))) i += 1;
  return i;
}

function skipExistingMask(text: string, from: number): number {
  let i = from;
  while (i < text.length && text[i] === "*") i += 1;
  while (i < text.length && isTokenChar(text.charCodeAt(i))) i += 1;
  return i;
}

function isTokenChar(c: number): boolean {
  // 0-9
  if (c >= 0x30 && c <= 0x39) return true;
  // A-Z
  if (c >= 0x41 && c <= 0x5a) return true;
  // a-z
  if (c >= 0x61 && c <= 0x7a) return true;
  // - or _
  return c === 0x2d || c === 0x5f;
}

function mask(token: string): string {
  // Mirrors the Rust threshold: anything <= 12 chars (length of the
  // bare `sk-ant-XXXXX` shape with no useful entropy beyond the
  // prefix) is fully masked. Longer tokens show their last 4 chars,
  // which match the Anthropic console's own truncation pattern.
  if (token.length <= 12) return "sk-ant-***";
  const last4 = token.slice(-4);
  return `sk-ant-***${last4}`;
}
