import { useEffect, useState } from "react";
import { Button } from "../../components/primitives/Button";
import { api } from "../../api";
import type {
  AutomationCreateDto,
  AutomationDetailsDto,
  OutputFormat,
  PermissionMode,
  PlatformOptionsDto,
  RouteSummaryDto,
  SchedulerCapabilitiesDto,
} from "../../types";
import { CronInput } from "./CronInput";

interface AutomationFormProps {
  /** When provided, the form is in edit mode and shows current values. */
  initial?: AutomationDetailsDto;
  routes: RouteSummaryDto[];
  capabilities: SchedulerCapabilitiesDto | null;
  busy: boolean;
  submitLabel: string;
  onSubmit: (dto: AutomationCreateDto) => void;
  onCancel: () => void;
}

const PERMISSION_MODES: PermissionMode[] = [
  "default",
  "acceptEdits",
  "bypassPermissions",
  "dontAsk",
  "plan",
  "auto",
];

const OUTPUT_FORMATS: OutputFormat[] = ["json", "text", "stream-json"];

const CWD_HINT =
  "CC discovers .claude (commands, agents, skills) from cwd up to git-root. Set this to your project root for slash-commands to work.";

/**
 * One form for create + edit. Edit mode disables the `name` field
 * (name is the URL-safe slug; renames would invalidate scheduler
 * registrations and run history, so we don't support them in v1).
 */
export function AutomationForm({
  initial,
  routes,
  capabilities,
  busy,
  submitLabel,
  onSubmit,
  onCancel,
}: AutomationFormProps) {
  const [name, setName] = useState(initial?.summary.name ?? "");
  const [displayName, setDisplayName] = useState(
    initial?.summary.display_name ?? "",
  );
  const [description, setDescription] = useState(
    initial?.summary.description ?? "",
  );
  const [binaryKind, setBinaryKind] = useState<"first_party" | "route">(
    initial?.summary.binary_kind ?? "first_party",
  );
  const [routeId, setRouteId] = useState<string>(
    initial?.summary.binary_route_id ?? "",
  );
  const [model, setModel] = useState(initial?.summary.model ?? "sonnet");
  const [cwd, setCwd] = useState(initial?.summary.cwd ?? "");
  const [prompt, setPrompt] = useState(initial?.prompt ?? "");
  const [systemPrompt, setSystemPrompt] = useState(
    initial?.system_prompt ?? "",
  );
  const [appendSystemPrompt, setAppendSystemPrompt] = useState(
    initial?.append_system_prompt ?? "",
  );
  const [permissionMode, setPermissionMode] = useState<PermissionMode>(
    (initial?.summary.permission_mode as PermissionMode) ??
      "bypassPermissions",
  );
  const [allowedToolsText, setAllowedToolsText] = useState(
    (initial?.summary.allowed_tools ?? ["Read", "Grep", "Glob"]).join(", "),
  );
  const [maxBudget, setMaxBudget] = useState<string>(
    String(initial?.summary.max_budget_usd ?? 0.5),
  );
  const [fallbackModel, setFallbackModel] = useState(
    initial?.fallback_model ?? "haiku",
  );
  const [outputFormat, setOutputFormat] = useState<OutputFormat>(
    (initial?.output_format as OutputFormat) ?? "json",
  );
  const [bareMode, setBareMode] = useState(initial?.bare ?? false);
  const [cron, setCron] = useState(initial?.summary.cron ?? "0 9 * * *");
  const [cronValid, setCronValid] = useState(true);
  const [platformOptions, setPlatformOptions] = useState<PlatformOptionsDto>(
    initial?.platform_options ?? {
      wake_to_run: false,
      catch_up_if_missed: true,
      run_when_logged_out: false,
    },
  );
  const [nameError, setNameError] = useState<string | null>(null);

  // Live name validation (skipped in edit mode — name is locked).
  useEffect(() => {
    if (initial) return;
    if (!name) {
      setNameError(null);
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        const v = await api.automationsValidateName(name);
        if (!cancelled) {
          setNameError(v.valid ? null : v.error ?? "invalid name");
        }
      } catch (e) {
        if (!cancelled) setNameError(String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [name, initial]);

  // Allowed-tools tokenizer: split on commas and whitespace, but
  // preserve patterns like `Bash(git *)` whose parentheses contain
  // their own spaces. We walk character by character, tracking
  // paren depth, and only break on top-level separators.
  const allowedTools = parseAllowedTools(allowedToolsText);

  const bypassWithoutTools =
    permissionMode === "bypassPermissions" && allowedTools.length === 0;

  // Budget: empty string = "no cap" (null), otherwise must be a
  // finite non-negative number. NaN/negative are rejected before
  // the DTO crosses IPC.
  const budgetTrimmed = maxBudget.trim();
  const budgetParsed: number | null =
    budgetTrimmed === "" ? null : Number(budgetTrimmed);
  const budgetInvalid =
    budgetParsed !== null && (!Number.isFinite(budgetParsed) || budgetParsed < 0);

  const canSubmit =
    !!name &&
    !!cwd &&
    !!prompt &&
    !!cron &&
    cronValid &&
    !nameError &&
    !bypassWithoutTools &&
    !budgetInvalid &&
    !busy &&
    (binaryKind === "first_party" || !!routeId);

  function handleSubmit() {
    const dto: AutomationCreateDto = {
      name: name.trim(),
      display_name: displayName.trim() || null,
      description: description.trim() || null,
      // In edit mode binary cannot change (the scheduler artifact
      // and shim are tied to the original binary kind). The form
      // disables the binary fields visually; here we belt-and-
      // braces it by preserving `initial.summary.binary_kind` /
      // `binary_route_id` when present.
      binary_kind: initial?.summary.binary_kind ?? binaryKind,
      binary_route_id: initial
        ? initial.summary.binary_route_id
        : binaryKind === "route"
          ? routeId
          : null,
      model: model.trim() || null,
      cwd: cwd.trim(),
      prompt,
      system_prompt: systemPrompt.trim() || null,
      append_system_prompt: appendSystemPrompt.trim() || null,
      permission_mode: permissionMode,
      allowed_tools: allowedTools,
      // Preserve initial values for fields the form doesn't yet
      // expose; otherwise an edit would silently clear them.
      add_dir: initial?.add_dir ?? [],
      max_budget_usd: budgetParsed,
      fallback_model: fallbackModel.trim() || null,
      output_format: outputFormat,
      json_schema: initial?.json_schema ?? null,
      bare: bareMode,
      extra_env: initial?.extra_env ?? {},
      cron: cron.trim(),
      timezone: initial?.summary.timezone ?? null,
      platform_options: platformOptions,
      log_retention_runs: initial?.log_retention_runs ?? 50,
    };
    onSubmit(dto);
  }

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-12)",
      }}
    >
      {/* Identity */}
      <Group title="Identity">
        <Field label="Name (a-z, 0-9, dash; 1-64; permanent)">
          <input
            type="text"
            value={name}
            disabled={!!initial || busy}
            onChange={(e) => setName(e.target.value)}
            placeholder="morning-pr-summary"
            style={inputStyle(!!nameError)}
          />
          {nameError && <Hint kind="error">{nameError}</Hint>}
        </Field>
        <Field label="Display name (optional)">
          <input
            type="text"
            value={displayName}
            disabled={busy}
            onChange={(e) => setDisplayName(e.target.value)}
            style={inputStyle()}
          />
        </Field>
        <Field label="Description (optional)">
          <input
            type="text"
            value={description}
            disabled={busy}
            onChange={(e) => setDescription(e.target.value)}
            style={inputStyle()}
          />
        </Field>
      </Group>

      {/* What it runs */}
      <Group title="What it runs">
        <Field label={initial ? "Binary (locked in edit mode)" : "Binary"}>
          <select
            value={binaryKind}
            disabled={busy || !!initial}
            onChange={(e) =>
              setBinaryKind(e.target.value as "first_party" | "route")
            }
            style={inputStyle()}
          >
            <option value="first_party">First-party `claude`</option>
            <option value="route">Third-party route</option>
          </select>
          {binaryKind === "route" && (
            <select
              value={routeId}
              disabled={busy || !!initial}
              onChange={(e) => setRouteId(e.target.value)}
              style={{ ...inputStyle(), marginTop: "var(--sp-6)" }}
            >
              <option value="">— select a route —</option>
              {routes.map((r) => (
                <option key={r.id} value={r.id}>
                  {r.name} ({r.provider_kind})
                </option>
              ))}
            </select>
          )}
          {initial && (
            <Hint>
              Binary cannot change in edit mode — the scheduler
              registration and helper shim are tied to the original
              binary. Delete and re-create to switch.
            </Hint>
          )}
        </Field>
        <Field label="Working directory (cwd)">
          <input
            type="text"
            value={cwd}
            disabled={busy}
            onChange={(e) => setCwd(e.target.value)}
            placeholder="/Users/me/github/myproject"
            style={inputStyle()}
          />
          <Hint>{CWD_HINT}</Hint>
        </Field>
        <Field label="Prompt">
          <textarea
            value={prompt}
            disabled={busy}
            onChange={(e) => setPrompt(e.target.value)}
            rows={4}
            placeholder="summarize today's PRs..."
            style={{ ...inputStyle(), resize: "vertical", minHeight: "5rem" }}
          />
        </Field>
        <Field label="System prompt (optional, replaces default)">
          <textarea
            value={systemPrompt}
            disabled={busy}
            onChange={(e) => setSystemPrompt(e.target.value)}
            rows={2}
            style={{ ...inputStyle(), resize: "vertical", minHeight: "3rem" }}
          />
        </Field>
        <Field label="Append to system prompt (optional)">
          <textarea
            value={appendSystemPrompt}
            disabled={busy}
            onChange={(e) => setAppendSystemPrompt(e.target.value)}
            rows={2}
            style={{ ...inputStyle(), resize: "vertical", minHeight: "3rem" }}
          />
        </Field>
      </Group>

      {/* How it runs */}
      <Group title="How it runs">
        <Field label="Model">
          <input
            type="text"
            value={model}
            disabled={busy}
            onChange={(e) => setModel(e.target.value)}
            placeholder="haiku | sonnet | opus | claude-sonnet-4-6"
            style={inputStyle()}
          />
        </Field>
        <Field label="Fallback model (when default is overloaded)">
          <input
            type="text"
            value={fallbackModel}
            disabled={busy}
            onChange={(e) => setFallbackModel(e.target.value)}
            placeholder="haiku"
            style={inputStyle()}
          />
        </Field>
        <Field label="Permission mode">
          <select
            value={permissionMode}
            disabled={busy}
            onChange={(e) => setPermissionMode(e.target.value as PermissionMode)}
            style={inputStyle()}
          >
            {PERMISSION_MODES.map((m) => (
              <option key={m} value={m}>
                {m}
              </option>
            ))}
          </select>
          <Hint>
            Unattended runs typically need bypassPermissions with a tight
            allowed-tools whitelist below.
          </Hint>
        </Field>
        <Field label="Allowed tools (comma- or space-separated)">
          <input
            type="text"
            value={allowedToolsText}
            disabled={busy}
            onChange={(e) => setAllowedToolsText(e.target.value)}
            placeholder="Read Grep Glob Bash(git *)"
            spellCheck={false}
            style={{
              ...inputStyle(bypassWithoutTools),
              fontFamily: "var(--ff-mono)",
            }}
          />
          {bypassWithoutTools && (
            <Hint kind="error">
              bypassPermissions requires a non-empty whitelist.
            </Hint>
          )}
        </Field>
        <Field label="Max budget (USD per run; empty = unlimited)">
          <input
            type="number"
            step="0.01"
            min="0"
            value={maxBudget}
            disabled={busy}
            onChange={(e) => setMaxBudget(e.target.value)}
            style={inputStyle(budgetInvalid)}
          />
          {budgetInvalid && (
            <Hint kind="error">
              Budget must be a finite non-negative number.
            </Hint>
          )}
        </Field>
        <Field label="Output format">
          <select
            value={outputFormat}
            disabled={busy}
            onChange={(e) => setOutputFormat(e.target.value as OutputFormat)}
            style={inputStyle()}
          >
            {OUTPUT_FORMATS.map((f) => (
              <option key={f} value={f}>
                {f}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Minimal mode (--bare)">
          <Toggle
            checked={bareMode}
            onChange={setBareMode}
            disabled={busy}
            label="Skip hooks, plugin sync, attribution, auto-memory, keychain reads, CLAUDE.md auto-discovery"
          />
        </Field>
      </Group>

      {/* When it runs */}
      <Group title="When it runs">
        <CronInput
          value={cron}
          onChange={setCron}
          onValidityChange={setCronValid}
          disabled={busy}
        />
      </Group>

      {/* Platform behavior */}
      <Group
        title={`Platform behavior (${capabilities?.native_label ?? "—"})`}
      >
        <Toggle
          checked={platformOptions.wake_to_run}
          onChange={(v) =>
            setPlatformOptions({ ...platformOptions, wake_to_run: v })
          }
          disabled={busy || !capabilities?.wake_to_run}
          label={`Wake the computer to run this${
            capabilities?.wake_to_run ? "" : " (not supported on this OS)"
          }`}
        />
        <Toggle
          checked={platformOptions.catch_up_if_missed}
          onChange={(v) =>
            setPlatformOptions({ ...platformOptions, catch_up_if_missed: v })
          }
          disabled={busy || !capabilities?.catch_up_if_missed}
          label={`Catch up if a run was missed${
            capabilities?.catch_up_if_missed
              ? ""
              : " (not supported on this OS)"
          }`}
        />
        <Toggle
          checked={platformOptions.run_when_logged_out}
          onChange={(v) =>
            setPlatformOptions({
              ...platformOptions,
              run_when_logged_out: v,
            })
          }
          disabled={busy || !capabilities?.run_when_logged_out}
          label={`Run while logged out${
            capabilities?.run_when_logged_out
              ? ""
              : " (not supported on this OS)"
          }`}
        />
      </Group>

      <div
        style={{
          display: "flex",
          gap: "var(--sp-8)",
          justifyContent: "flex-end",
          marginTop: "var(--sp-8)",
        }}
      >
        <Button variant="ghost" onClick={onCancel} disabled={busy}>
          Cancel
        </Button>
        <Button
          variant="solid"
          onClick={handleSubmit}
          disabled={!canSubmit}
        >
          {submitLabel}
        </Button>
      </div>
    </div>
  );
}

// ---------- presentation helpers ----------

function Group({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <fieldset
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-8)",
        padding: "var(--sp-12)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-3)",
        background: "var(--bg-raised)",
        margin: 0,
      }}
    >
      <legend
        style={{
          fontSize: "var(--fs-xs)",
          color: "var(--fg-2)",
          padding: "0 var(--sp-4)",
        }}
      >
        {title}
      </legend>
      {children}
    </fieldset>
  );
}

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <label
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-4)",
        fontSize: "var(--fs-xs)",
        color: "var(--fg-2)",
      }}
    >
      <span>{label}</span>
      {children}
    </label>
  );
}

function Hint({
  kind = "info",
  children,
}: {
  kind?: "info" | "error";
  children: React.ReactNode;
}) {
  return (
    <span
      style={{
        fontSize: "var(--fs-2xs)",
        color: kind === "error" ? "var(--danger)" : "var(--fg-3)",
      }}
    >
      {children}
    </span>
  );
}

function Toggle({
  checked,
  onChange,
  disabled,
  label,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
  label: string;
}) {
  return (
    <label
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-6)",
        fontSize: "var(--fs-xs)",
        color: disabled ? "var(--fg-3)" : "var(--fg-2)",
        cursor: disabled ? "not-allowed" : "pointer",
      }}
    >
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span>{label}</span>
    </label>
  );
}

/**
 * Tokenize an allowed-tools field. Splits on top-level commas and
 * whitespace, but keeps parenthesized arg patterns intact:
 *
 *   "Read, Grep, Bash(git *), Bash(cat *)" →
 *     ["Read", "Grep", "Bash(git *)", "Bash(cat *)"]
 *
 * Naive `split(/[,\s]+/)` would shred `Bash(git *)` into
 * `Bash(git`, `*)` — silently dropping the whole permission
 * pattern from the whitelist.
 */
function parseAllowedTools(input: string): string[] {
  const out: string[] = [];
  let current = "";
  let depth = 0;
  for (const ch of input) {
    if (ch === "(") {
      depth += 1;
      current += ch;
    } else if (ch === ")") {
      depth = Math.max(0, depth - 1);
      current += ch;
    } else if ((ch === "," || /\s/.test(ch)) && depth === 0) {
      const trimmed = current.trim();
      if (trimmed) out.push(trimmed);
      current = "";
    } else {
      current += ch;
    }
  }
  const trimmed = current.trim();
  if (trimmed) out.push(trimmed);
  return out;
}

function inputStyle(invalid: boolean = false): React.CSSProperties {
  return {
    fontSize: "var(--fs-sm)",
    padding: "var(--sp-6) var(--sp-8)",
    border: `var(--bw-hair) solid ${
      invalid ? "var(--danger)" : "var(--line)"
    }`,
    borderRadius: "var(--r-2)",
    background: "var(--bg-raised)",
    color: "var(--fg)",
  };
}
