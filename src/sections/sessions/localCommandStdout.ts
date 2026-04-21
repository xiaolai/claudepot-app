/**
 * UI mirror of the Rust `extract_local_command_stdout` helper. CC wraps
 * slash-command output in a `<local-command-stdout>...</local-command-stdout>`
 * tag — metadata we want to hide from the rendered bubble.
 *
 * Returns the unwrapped payload when the tag is present; returns the
 * original string unchanged otherwise.
 */
export function stripLocalCommandStdout(text: string): string {
  const open = "<local-command-stdout>";
  const close = "</local-command-stdout>";
  const s = text.indexOf(open);
  if (s < 0) return text;
  const rest = text.slice(s + open.length);
  const e = rest.indexOf(close);
  if (e < 0) return text;
  return rest.slice(0, e);
}
