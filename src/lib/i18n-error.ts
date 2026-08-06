// The ONE rendering path from a thrown/rejected value to a
// user-visible string. Consolidates what used to be three
// partially-overlapping layers (lib/toastError.ts, lib/errors.ts,
// and ~97 raw `${e}` template literals at pushToast call sites) so
// the i18n error-code work (plan §2.5 / P5) lands in exactly one
// file: when backend errors start carrying `{ code, params, message }`,
// this module resolves the code against the `errors.*` catalog and
// every call site localizes at once, with `message` as the English
// fallback.
//
// P1 shape: extract → redact → truncate → optional scope prefix.
// P5 adds the first step: resolve `ErrorDto.code` against the `errors`
// catalog, falling back to `message` (Rust's English) when the code has
// no sentence. That fallback is what let the Rust fan-out land without a
// frontend flag day, and it is also why the catalog completeness lock in
// `errorCodes.test.ts` has to be a test — a missing entry silently
// renders English rather than throwing.

import i18n from "./i18n";
import { redactSecrets } from "./redactSecrets";

/**
 * Hard cap for toast text. Toasts live in a small floating row at
 * the bottom of the window; a 1 KB stack trace would push the
 * primary action off-screen and the user can't read past the first
 * line anyway. 240 chars is enough for "Sync failed: 401 Unauthorized
 * (refresh_token expired — sign in again)" plus a little headroom.
 */
const MAX_TOAST_LEN = 240;

/** Pull a readable string out of any thrown/rejected value, never
 *  surfacing the useless `"[object Object]"`. Tauri command
 *  rejections are plain strings (`Result<T, String>`); IPC-level
 *  failures (command not registered, arg deserialization) reject
 *  with an `Error` object. */
export function extractMessage(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  if (
    e != null &&
    typeof e === "object" &&
    "message" in e &&
    typeof (e as { message: unknown }).message === "string"
  ) {
    return (e as { message: string }).message;
  }
  if (e == null) return "";
  const s = String(e);
  return s === "[object Object]" ? "" : s;
}

/** The `{ code, params, message }` shape a migrated Tauri command
 *  rejects with (`src-tauri/src/dto_error.rs`). Duck-typed rather than
 *  branded: a command that has not been migrated yet rejects with a
 *  plain string and must keep working. */
interface CodedError {
  code: string;
  params?: Record<string, unknown>;
}

function asCoded(e: unknown): CodedError | null {
  if (e == null || typeof e !== "object") return null;
  const o = e as Record<string, unknown>;
  if (typeof o.code !== "string" || o.code.length === 0) return null;
  const params =
    o.params != null && typeof o.params === "object" && !Array.isArray(o.params)
      ? (o.params as Record<string, unknown>)
      : undefined;
  return { code: o.code, params };
}

/**
 * The `<module>.<variant>` identity of a backend rejection, or `null`
 * for a value that carries none (a plain-string rejection from a
 * command that has not migrated, or a JS-side `Error`).
 *
 * Exported so a caller that has to *branch* on which failure it was
 * does it on the code rather than on the rendered sentence. Matching
 * `renderError(e)` for a substring works only while the sentence is
 * English — the moment the catalog gains a zh-CN entry, the branch
 * silently stops firing. That is not hypothetical: the "Sign in again"
 * banner keyed off the literal `auth rejected:` prefix that
 * `sync_from_current_cc` used to compose.
 */
export function errorCode(e: unknown): string | null {
  return asCoded(e)?.code ?? null;
}

/**
 * The localized sentence for a backend error code, or `null` when the
 * value carries no code or the catalog has no entry for it.
 *
 * Returning `null` rather than i18next's key-echo is the point: the
 * caller falls back to `ErrorDto.message`, the English Rust already
 * composed, so an untranslated code degrades to the exact text the user
 * saw before this phase rather than to a raw `errors:foo.bar`.
 */
/**
 * Resolve a nested cause into a localized clause.
 *
 * A wrapper error whose *phase* is the user-facing fact keeps its own
 * code (`route_save.store` means "the previously-saved route is still
 * active") while naming what actually failed underneath. That cause
 * arrives as `cause_code` + `cause_params`, never as rendered English —
 * splicing `detail` in produced half-translated sentences like
 * "存储密钥失败：keychain write denied".
 *
 * Returns `null` when there is no cause or its code has no sentence, so
 * the wrapper's own sentence can fall back rather than interpolate the
 * literal `{{cause}}`.
 */
function causeClause(params: Record<string, unknown> | undefined): string | null {
  const code = params?.cause_code;
  if (typeof code !== "string" || !code) return null;
  if (!i18n.exists(code, { ns: "errors" })) return null;
  const nested =
    params?.cause_params && typeof params.cause_params === "object"
      ? (params.cause_params as Record<string, unknown>)
      : {};
  return (i18n.t as unknown as (k: string, o?: object) => string)(code, {
    ...nested,
    ns: "errors",
  });
}

function catalogSentence(e: unknown): string | null {
  const coded = asCoded(e);
  if (!coded) return null;
  // An error code is a value from Rust, not a literal in this codebase,
  // so typed `t()` cannot check it. Same sanctioned-dynamic-lookup shape
  // as `lib/notifications/labels.ts`; the completeness check that typed
  // keys would give is `errorCodes.test.ts` instead, run against the
  // Rust registry rather than against the TS catalog alone.
  const key = coded.code;
  if (!i18n.exists(key, { ns: "errors" })) return null;
  const cause = causeClause(coded.params);
  if (cause === null && typeof coded.params?.cause_code === "string") {
    // The wrapper names a cause the catalog cannot render. Its own
    // sentence interpolates `{{cause}}`, so rendering it now would show
    // the user a literal brace pair — fall back to English instead.
    return null;
  }
  // `params` is a flat JSON object built by Rust's `params()`; it never
  // carries a secret (locked by the `*_params_never_carry_a_token`
  // tests) and it is the only interpolation source — the English
  // `message` is never spliced into a localized sentence.
  return (i18n.t as unknown as (k: string, o?: object) => string)(key, {
    ...(coded.params ?? {}),
    ...(cause !== null ? { cause } : {}),
    ns: "errors",
  });
}

/**
 * Render any thrown/rejected value for a user surface, redacting
 * `sk-ant-*` tokens and bounding the length. The redaction rule from
 * `rules/rust-conventions.md` applies to JS toasts as much as log
 * lines — an unredacted `${e}` happily renders an oauth blob if the
 * backend echoes one back, and it sits in the DOM until the toast
 * dismisses.
 *
 * With `scope`: `<scope>: <redacted, truncated message>` — scope is a
 * short verb phrase ("Sync failed", "Verify all") naming what was
 * being attempted. Callers own the scope's language; this module owns
 * the message's.
 *
 * Redaction runs on the *resolved* string, catalog sentence included,
 * and on `scope`. A localized sentence interpolates `params`, and while
 * Rust guarantees no token reaches `params`, the guarantee is enforced
 * on the other side of an IPC boundary — running the sweep here costs
 * nothing and removes the need to trust it twice.
 *
 * `scope` is redacted and length-capped along with everything else.
 * It reads like a fixed verb phrase, but callers interpolate values
 * into it (`renderError(e, \`Couldn't update ${key}\`)`), so treating it
 * as trusted would leave a hole exactly where the message path is
 * sealed — and it would also let a long scope push the composed string
 * past the cap the toast row is sized for.
 */
export function renderError(e: unknown, scope?: string): string {
  // The floor. `extractMessage` returns "" for a thrown value with no
  // readable text, and an empty string renders as a *missing* banner
  // (`{err && <Banner/>}`) — the operation failed and the surface said
  // nothing. Inherited from `lib/errors.ts`, which is gone; a Tauri
  // rejection should never reach it (`ErrorDto.message` is never empty
  // by contract) but "should never" is exactly what a floor is for.
  const raw = catalogSentence(e) ?? extractMessage(e);
  const body = redactSecrets(raw) || i18n.t("ui.unknownError");
  const composed = scope ? `${redactSecrets(scope)}: ${body}` : body;
  return composed.length <= MAX_TOAST_LEN
    ? composed
    : `${composed.slice(0, MAX_TOAST_LEN - 1)}…`;
}

/**
 * Convenience wrapper that pushes a redacted error toast through the
 * caller-supplied `pushToast` setter. One redaction pipeline for
 * every ad-hoc handler — a future change to the rules (masking new
 * token shapes, code→catalog resolution) lands in exactly one place.
 */
export function toastError(
  pushToast: (kind: "info" | "error", text: string) => void,
  scope: string,
  e: unknown,
): void {
  pushToast("error", renderError(e, scope));
}
