// Shapes for Global → Config → Env Variables. Mirrors
// `claudepot_core::cc_env` (returned verbatim by the `cc_env_*`
// commands — no hand-written DTO layer, same call as cc-retention).
//
// The one rule this file exists to encode: a value that could be a
// credential never has a `value` field to read. `EnvValue` is a tagged
// union precisely so "set but withheld" is a state the renderer can
// display and cannot accidentally print.

export type EnvControl = "toggle" | "enum" | "number" | "text";

/**
 * A specific, evidenced way a value can hurt you. The first three come
 * from Claude Code's own taxonomy; `disable_updates` and `execute_code`
 * are Claudepot's, for classes CC does not enumerate.
 *
 * `unknown` is the honest label for "not on CC's pre-trust allowlist,
 * specific risk not established" — it gets a muted note, never an
 * invented risk label.
 */
export type Hazard =
  | "redirect"
  | "trust_cert"
  | "execute_code"
  | "switch_project"
  | "disable_updates"
  | "unknown";

/** Why a variable is not editable here. */
export type Blocked =
  /** CC reads it before settings load, so writing it splits its bootstrap. */
  | "bootstrap_split_brain"
  /** Injected per run by whatever launched CC. */
  | "host_injected"
  /**
   * CC reads it only from the environment `claude` was started with, never
   * from a settings `env` block — so writing it here would change nothing
   * while reporting success.
   */
  | "env_only_not_settings";

/**
 * Independent attributes, deliberately not exclusive tiers — CC's own
 * sets overlap, and they answer different questions. `pretrust_safe`
 * means "CC would apply this from an untrusted source"; `secret` means
 * "this pane must never display it". `ANTHROPIC_CUSTOM_HEADERS` is both.
 */
export interface EnvSafety {
  secret: boolean;
  blocked_reason: Blocked | null;
  pretrust_safe: boolean;
  provider_managed: boolean;
  hazards: Hazard[];
}

export interface EnvVarSpec {
  name: string;
  category: string;
  /** The official prose, verbatim. */
  doc: string;
  /** Valid only when `crosscheck_is_exact`. */
  present_in_build: boolean;
  safety: EnvSafety;
  control: EnvControl;
  values: string[] | null;
  /** Shown as a placeholder, never written — see "Restore default". */
  default: string;
  unit: string;
  on: string | null;
  /** `"0"` → three-state (Unset · 0 · 1). `"unset"` → two-state. */
  off: string | null;
  numeric_evidence: string;
  /** Display hint only. Never validation. */
  format: string;
}

export type EnvValueKind =
  | "string"
  | "number"
  | "bool"
  | "array"
  | "object"
  | "null";

export type EnvValue =
  | { state: "absent" }
  /** Set, and credential-capable. There is no value to read. */
  | { state: "secret_set" }
  | { state: "known"; value: string }
  /** A scalar the control cannot round-trip. Read-only + Replace/Clear. */
  | { state: "custom"; raw: string; kind: EnvValueKind }
  /** An array/object/null. Same, minus the value. */
  | { state: "custom_opaque"; kind: EnvValueKind }
  /** A key in nobody's list. Value withheld — it may be a future credential. */
  | { state: "withheld"; kind: EnvValueKind };

export type ResolvedSource =
  | "settings_override"
  | "legacy_global"
  /** NOT "CC default" — a shell export beats both files and is invisible. */
  | "no_known_file_override";

export interface EnvVarState {
  spec: EnvVarSpec;
  settings_value: EnvValue;
  legacy_global: EnvValue | null;
  resolved_source: ResolvedSource;
}

export interface UnrecognizedEntry {
  name: string;
  /** Always `withheld`. */
  value: EnvValue;
}

export type UndocumentedBucket =
  | { state: "available"; snapshot_version: string; names: string[] }
  | {
      state: "unavailable";
      snapshot_version: string;
      installed_version: string | null;
    };

export interface EnvCategory {
  key: string;
  label: string;
}

export interface EnvOverview {
  documented: EnvVarState[];
  undocumented: UndocumentedBucket;
  unrecognized: UnrecognizedEntry[];
  /** Category keys and labels in grouping order, straight from the
   *  generated spec — not mirrored here, so adding or reordering a category
   *  is one edit rather than two that can disagree. */
  categories: EnvCategory[];
  docs_fetched_at: string;
  docs_sha256: string;
  binary_crosscheck_version: string;
  installed_version: string | null;
  installed_path: string | null;
  /** The settings file this pane actually edits, resolved. Not a constant
   *  `~/.claude/settings.json`: CLAUDE_CONFIG_DIR moves it, and telling a
   *  user to hand-edit a file that is not the one being written is worse
   *  than saying nothing. */
  settings_path: string;
  crosscheck_is_exact: boolean;
}
