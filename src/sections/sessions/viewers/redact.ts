/**
 * UI-side mirror of `claudepot_core::session_export::redact_secrets`.
 *
 * The exporter runs the Rust version before crossing the Tauri
 * boundary, but the chunk / transcript renderers receive data via
 * `session_chunks` / `session_linked_tools` which is *not* redacted
 * because those endpoints are authoritative source-of-truth for live
 * debugging. Apply this helper at every leaf that renders a string
 * the user didn't type themselves.
 *
 * Matches `sk-ant-<tokenchars>` where tokenchars are alphanumerics,
 * `-`, or `_` — the same shape the Rust side enforces. Short tokens
 * (<= 12 chars) are masked completely; longer tokens keep their last
 * four characters so readers can still tell two leaks apart.
 */
const TOKEN_RE = /sk-ant-[A-Za-z0-9_-]+/g;

export function redactSecrets(text: string | null | undefined): string {
  if (!text) return text ?? "";
  if (!text.includes("sk-ant-")) return text;
  return text.replace(TOKEN_RE, (tok) => {
    if (tok.length <= 12) return "sk-ant-***";
    return `sk-ant-***${tok.slice(-4)}`;
  });
}
