// Memory-health wire types. Mirror
// `src-tauri/src/commands_memory_health::*Dto` byte-for-byte.

/** One file's audit metrics. `missing: true` means the file wasn't
 *  on disk — distinct from "exists but empty" so the consumer can
 *  render a muted "not configured" tile. */
export interface FileHealth {
  /** Absolute path the metrics were computed against. */
  path: string;
  missing: boolean;
  line_count: number;
  char_count: number;
  /** Physical lines past the global truncation cutoff (≥201).
   *  Non-zero values are the actionable signal — CC literally
   *  cannot see those lines. */
  lines_past_cutoff: number;
  chars_past_cutoff: number;
  /** Approximate token count (`char_count / 4`, rounded). Labeled
   *  `est` everywhere because a precise tokenizer would inflate
   *  binary size for marginal gain. */
  est_tokens: number;
}

export interface MemoryHealthReport {
  claude_md: FileHealth;
  memory_md: FileHealth;
  /** CC's truncation cutoff at the time the report was built. The
   *  consumer renders this in the "lines past N" tile so the
   *  warning is self-explanatory. */
  line_cutoff: number;
}
