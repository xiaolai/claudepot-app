// Path-string helpers for display. Windows-aware: a path may use `\`
// separators (see .claude/rules/paths.md — every string that might be a
// path is a Windows path until proven otherwise). Callers that need to
// *copy* the full path pair these with <CopyButton text={fullPath} /> and
// a `title={fullPath}` per .claude/rules/path-display.md.

/** Last path segment, splitting on both `/` and `\`. Returns the input
 *  unchanged when it has no separator (already a basename) or is empty. */
export function basename(p: string): string {
  const parts = p.split(/[/\\]/).filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1]! : p;
}
