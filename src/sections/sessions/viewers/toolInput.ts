/**
 * Shared helpers for parsing a `LinkedTool.input_preview` into a typed
 * shape specific to a known tool name. The previews are captured by
 * `truncate_prompt` in Rust and are valid JSON in most cases, but
 * might be clipped at 240 chars — so every parser defends against
 * missing fields and treats "couldn't parse" as "fall back to the raw
 * preview".
 */

export type ParseResult<T> = { ok: true; value: T } | { ok: false; raw: string };

export function parseToolInput<T>(preview: string): ParseResult<T> {
  try {
    return { ok: true, value: JSON.parse(preview) as T };
  } catch {
    return { ok: false, raw: preview };
  }
}

export interface ReadInput {
  file_path?: string;
  offset?: number;
  limit?: number;
  pages?: string;
}

export interface EditInput {
  file_path?: string;
  old_string?: string;
  new_string?: string;
  replace_all?: boolean;
}

export interface WriteInput {
  file_path?: string;
  content?: string;
}

export interface BashInput {
  /** Canonical field name in live CC transcripts. */
  cmd?: string;
  /**
   * Legacy/alternate name seen in some older transcripts and in the
   * claude-devtools documentation. The viewer prefers `cmd` but
   * falls back to `command` if `cmd` is missing.
   */
  command?: string;
  description?: string;
  timeout?: number;
}

/** Resolve the command string from either spelling. */
export function bashCommand(input: BashInput): string {
  return input.cmd ?? input.command ?? "";
}

export interface BashResult {
  stdout?: string;
  stderr?: string;
  interrupted?: boolean;
  is_image?: boolean;
  exit_code?: number;
}

/** Try to interpret a tool result body as JSON; fall back to raw text. */
export function tryParseResult<T>(body: string): ParseResult<T> {
  const trimmed = body.trim();
  if (!trimmed.startsWith("{") && !trimmed.startsWith("[")) {
    return { ok: false, raw: body };
  }
  try {
    return { ok: true, value: JSON.parse(body) as T };
  } catch {
    return { ok: false, raw: body };
  }
}

/**
 * Compute a minimal unified-style diff for Edit viewer. We don't need
 * Myers' algorithm for a single-string replacement — a 2-column split
 * + a context band around the change is enough for the UI.
 */
export interface DiffLine {
  kind: "add" | "remove" | "context";
  text: string;
  /** 1-based old file line number, if the line exists in the old text. */
  oldLine?: number;
  /** 1-based new file line number, if the line exists in the new text. */
  newLine?: number;
}

export function computeDiff(
  oldString: string,
  newString: string,
  contextLines = 2,
): DiffLine[] {
  // `"".split("\n")` yields `[""]` which would show as a phantom
  // removed/added empty line. Normalize to "no lines" when the input
  // is truly empty.
  const oldLines = oldString === "" ? [] : oldString.split("\n");
  const newLines = newString === "" ? [] : newString.split("\n");
  const diff: DiffLine[] = [];
  // Common prefix / suffix trimming keeps the block focused on the edit.
  let prefix = 0;
  while (
    prefix < oldLines.length &&
    prefix < newLines.length &&
    oldLines[prefix] === newLines[prefix]
  ) {
    prefix++;
  }
  let suffix = 0;
  while (
    suffix < oldLines.length - prefix &&
    suffix < newLines.length - prefix &&
    oldLines[oldLines.length - 1 - suffix] ===
      newLines[newLines.length - 1 - suffix]
  ) {
    suffix++;
  }

  const showPrefix = Math.max(0, prefix - contextLines);
  const showSuffixStart = Math.max(prefix, oldLines.length - suffix);
  // Leading context
  for (let i = showPrefix; i < prefix; i++) {
    diff.push({
      kind: "context",
      text: oldLines[i],
      oldLine: i + 1,
      newLine: i + 1,
    });
  }
  // Removed lines
  for (let i = prefix; i < oldLines.length - suffix; i++) {
    diff.push({ kind: "remove", text: oldLines[i], oldLine: i + 1 });
  }
  // Added lines
  for (let i = prefix; i < newLines.length - suffix; i++) {
    diff.push({ kind: "add", text: newLines[i], newLine: i + 1 });
  }
  // Trailing context
  const tailEnd = Math.min(oldLines.length, showSuffixStart + contextLines);
  for (let i = showSuffixStart; i < tailEnd; i++) {
    const newIdx = newLines.length - (oldLines.length - i);
    diff.push({
      kind: "context",
      text: oldLines[i],
      oldLine: i + 1,
      newLine: newIdx + 1,
    });
  }
  return diff;
}
