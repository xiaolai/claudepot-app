import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/primitives/Button";
import { Tag } from "../../components/primitives/Tag";
import { api } from "../../api";
import { renderError } from "../../lib/i18n-error";
import type {
  AgentCreateDto,
  AgentDetailsDto,
  McpServerRef,
  OutputFormat,
  PermissionMode,
  PlatformOptionsDto,
  RouteSummaryDto,
  SchedulerCapabilitiesDto,
} from "../../types";
import { CronInput } from "./CronInput";

interface AgentFormProps {
  /** When provided, the form is in edit mode and shows current values. */
  initial?: AgentDetailsDto;
  routes: RouteSummaryDto[];
  capabilities: SchedulerCapabilitiesDto | null;
  busy: boolean;
  submitLabel: string;
  onSubmit: (dto: AgentCreateDto) => void;
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

/**
 * Trigger shape the form authors. `"event"` is v1's session-settled
 * reactive trigger (PRD §7); `"cron"` is the default; `"manual"`
 * is run-now-only.
 */
type FormTriggerKind = "cron" | "event" | "manual";

/**
 * Default debounce for a session-settled event trigger, in seconds.
 * Mirrors `claudepot_core::agent::DEFAULT_DEBOUNCE_SECS` (PRD §7.1).
 */
const DEFAULT_EVENT_DEBOUNCE_SECS = 600;

/**
 * One form for create + edit. Edit mode disables the `name` field
 * (name is the URL-safe slug; renames would invalidate scheduler
 * registrations and run history, so we don't support them in v1).
 */
export function AgentForm({
  initial,
  routes,
  capabilities,
  busy,
  submitLabel,
  onSubmit,
  onCancel,
}: AgentFormProps) {
  const { t } = useTranslation("agents");
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
  // ---- Agent-spec fields (Phase 1) ----
  const [disallowedToolsText, setDisallowedToolsText] = useState(
    (initial?.disallowed_tools ?? []).join(", "),
  );
  // MCP servers. Phase 1 surfaces a one-click "Attach Claudepot
  // memory" toggle; custom servers carried on an edited agent are
  // preserved verbatim but not editable in this form.
  const [mcpServers, setMcpServers] = useState<McpServerRef[]>(
    initial?.mcp_servers ?? [],
  );
  const [runAs, setRunAs] = useState(initial?.run_as ?? "");
  const [taskBudget, setTaskBudget] = useState<string>(
    initial?.task_budget != null ? String(initial.task_budget) : "",
  );
  const [rateMinInterval, setRateMinInterval] = useState<string>(
    initial?.rate_limit?.min_interval_secs != null
      ? String(initial.rate_limit.min_interval_secs)
      : "",
  );
  const [rateMaxPerDay, setRateMaxPerDay] = useState<string>(
    initial?.rate_limit?.max_per_day != null
      ? String(initial.rate_limit.max_per_day)
      : "",
  );
  const [cron, setCron] = useState(initial?.summary.cron ?? "0 9 * * *");
  const [cronValid, setCronValid] = useState(true);
  // Trigger-type selector. Reads the existing record's
  // `trigger_kind` in edit mode; defaults to "cron" on create so
  // the historical Add-Agent flow is unchanged.
  const [triggerKind, setTriggerKind] = useState<FormTriggerKind>(() => {
    const k = initial?.summary.trigger_kind;
    if (k === "event" || k === "manual") return k;
    return "cron";
  });
  // Debounce window for a session-settled event trigger. Stored
  // as a free-form string so the user can clear it; parsed below.
  const [eventDebounceSecs, setEventDebounceSecs] = useState<string>(() => {
    const secs = initial?.summary.event_debounce_secs;
    if (secs != null) return String(secs);
    return String(DEFAULT_EVENT_DEBOUNCE_SECS);
  });
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
        const v = await api.agentsValidateName(name);
        if (!cancelled) {
          setNameError(
            v.valid ? null : v.error ?? t("form.errors.invalidName"),
          );
        }
      } catch (e) {
        if (!cancelled) setNameError(renderError(e));
      }
    })();
    return () => {
      cancelled = true;
    };
    // `t` is a dep because the fallback message is localized. Re-running
    // on a language switch just re-validates the same name — idempotent,
    // and it relabels an on-screen error instead of stranding it in the
    // previous language.
  }, [name, initial, t]);

  // Allowed-tools tokenizer: split on commas and whitespace, but
  // preserve patterns like `Bash(git *)` whose parentheses contain
  // their own spaces. We walk character by character, tracking
  // paren depth, and only break on top-level separators.
  const allowedTools = parseAllowedTools(allowedToolsText);
  const disallowedTools = parseAllowedTools(disallowedToolsText);

  const bypassWithoutTools =
    permissionMode === "bypassPermissions" && allowedTools.length === 0;

  // Task budget: empty = "no ceiling" (null); otherwise a positive
  // integer token count. Zero / negative / non-finite are rejected.
  const taskBudgetTrimmed = taskBudget.trim();
  const taskBudgetParsed: number | null =
    taskBudgetTrimmed === "" ? null : Number(taskBudgetTrimmed);
  const taskBudgetInvalid =
    taskBudgetParsed !== null &&
    (!Number.isFinite(taskBudgetParsed) ||
      !Number.isInteger(taskBudgetParsed) ||
      taskBudgetParsed <= 0);

  // Rate limit: each field empty = "no limit". A populated field
  // must be a positive integer.
  const minIntervalTrimmed = rateMinInterval.trim();
  const minIntervalParsed: number | null =
    minIntervalTrimmed === "" ? null : Number(minIntervalTrimmed);
  const minIntervalInvalid =
    minIntervalParsed !== null &&
    (!Number.isInteger(minIntervalParsed) || minIntervalParsed <= 0);
  const maxPerDayTrimmed = rateMaxPerDay.trim();
  const maxPerDayParsed: number | null =
    maxPerDayTrimmed === "" ? null : Number(maxPerDayTrimmed);
  const maxPerDayInvalid =
    maxPerDayParsed !== null &&
    (!Number.isInteger(maxPerDayParsed) || maxPerDayParsed <= 0);
  const rateLimitInvalid = minIntervalInvalid || maxPerDayInvalid;
  /**
   * Event-trigger rate-limit guard (PRD D9). The store-side
   * invariant rejects an event-triggered agent that carries no
   * usable rate-limit; surface that here so the user sees the
   * problem before submit, not as a backend error toast.
   */
  const rateLimitMissingForEvent =
    triggerKind === "event" &&
    minIntervalParsed === null &&
    maxPerDayParsed === null;

  // Event-trigger debounce validation. An empty / non-positive
  // integer is rejected — the evaluator treats `debounce_secs`
  // as `u64`, and a debounce of 0 would fire while the session
  // was still active.
  const eventDebounceTrimmed = eventDebounceSecs.trim();
  const eventDebounceParsed: number | null =
    triggerKind === "event"
      ? eventDebounceTrimmed === ""
        ? null
        : Number(eventDebounceTrimmed)
      : null;
  const eventDebounceInvalid =
    triggerKind === "event" &&
    (eventDebounceParsed === null ||
      !Number.isFinite(eventDebounceParsed) ||
      !Number.isInteger(eventDebounceParsed) ||
      eventDebounceParsed <= 0);

  const memoryAttached = mcpServers.some(
    (s) => s.kind === "claudepot_memory",
  );
  const customMcpServers = mcpServers.filter((s) => s.kind === "custom");

  function toggleMemoryServer() {
    setMcpServers((prev) =>
      prev.some((s) => s.kind === "claudepot_memory")
        ? prev.filter((s) => s.kind !== "claudepot_memory")
        : [...prev, { kind: "claudepot_memory" }],
    );
  }

  // Lifecycle is read-only in the form. A new (un-`initial`) agent
  // is armed on create by the GUI flow, so it shows as "installed".
  const lifecycle = initial?.summary.lifecycle ?? "installed";

  // Budget: empty string = "no cap" (null), otherwise must be a
  // finite non-negative number. NaN/negative are rejected before
  // the DTO crosses IPC.
  const budgetTrimmed = maxBudget.trim();
  const budgetParsed: number | null =
    budgetTrimmed === "" ? null : Number(budgetTrimmed);
  const budgetInvalid =
    budgetParsed !== null && (!Number.isFinite(budgetParsed) || budgetParsed < 0);

  // Trigger gate: cron requires a valid cron expression; event
  // requires a positive debounce + a usable rate-limit; manual
  // has no per-tick gate (Run-Now is the only path).
  const triggerOk =
    triggerKind === "manual" ||
    (triggerKind === "cron" && !!cron && cronValid) ||
    (triggerKind === "event" &&
      !eventDebounceInvalid &&
      !rateLimitMissingForEvent);

  const canSubmit =
    !!name &&
    !!cwd &&
    !!prompt &&
    triggerOk &&
    !nameError &&
    !bypassWithoutTools &&
    !budgetInvalid &&
    !taskBudgetInvalid &&
    !rateLimitInvalid &&
    !busy &&
    (binaryKind === "first_party" || !!routeId);

  function handleSubmit() {
    const dto: AgentCreateDto = {
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
      // Trigger fields. Cron-mode keeps the historical wire shape
      // (`trigger_kind` omitted → defaults to "cron" on the
      // backend). Event-mode sets `trigger_kind = "event"` and
      // sends `event_kind` + `event_debounce_secs` (the cron
      // string is still serialized for back-compat but ignored).
      trigger_kind: triggerKind,
      event_kind:
        triggerKind === "event" ? "session_settled" : null,
      event_debounce_secs:
        triggerKind === "event"
          ? eventDebounceParsed ?? DEFAULT_EVENT_DEBOUNCE_SECS
          : null,
      cron: triggerKind === "cron" ? cron.trim() : "",
      timezone: initial?.summary.timezone ?? null,
      platform_options: platformOptions,
      log_retention_runs: initial?.log_retention_runs ?? 50,
      // ---- Agent-spec fields (Phase 1) ----
      disallowed_tools: disallowedTools,
      mcp_servers: mcpServers,
      run_as: runAs.trim() || null,
      task_budget: taskBudgetParsed,
      rate_limit:
        minIntervalParsed === null && maxPerDayParsed === null
          ? null
          : {
              min_interval_secs: minIntervalParsed,
              max_per_day: maxPerDayParsed,
            },
      // Audit field — only the (Phase 2) AI-drafting path sets this.
      drafted_by: null,
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
      <Group title={t("form.groups.identity")}>
        <Field label={t("form.fields.name")}>
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
        <Field label={t("form.fields.displayName")}>
          <input
            type="text"
            value={displayName}
            disabled={busy}
            onChange={(e) => setDisplayName(e.target.value)}
            style={inputStyle()}
          />
        </Field>
        <Field label={t("form.fields.description")}>
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
      <Group title={t("form.groups.what")}>
        <Field
          label={
            initial
              ? t("form.fields.binaryLocked")
              : t("form.fields.binary")
          }
        >
          <select
            value={binaryKind}
            disabled={busy || !!initial}
            onChange={(e) =>
              setBinaryKind(e.target.value as "first_party" | "route")
            }
            style={inputStyle()}
          >
            <option value="first_party">
              {t("form.options.binaryFirstParty")}
            </option>
            <option value="route">{t("form.options.binaryRoute")}</option>
          </select>
          {binaryKind === "route" && (
            <select
              value={routeId}
              disabled={busy || !!initial}
              onChange={(e) => setRouteId(e.target.value)}
              style={{ ...inputStyle(), marginTop: "var(--sp-6)" }}
            >
              <option value="">{t("form.options.selectRoute")}</option>
              {routes.map((r) => (
                <option key={r.id} value={r.id}>
                  {r.name} ({r.provider_kind})
                </option>
              ))}
            </select>
          )}
          {initial && (
            <Hint>{t("form.hints.binaryLocked")}</Hint>
          )}
        </Field>
        <Field label={t("form.fields.cwd")}>
          <input
            type="text"
            value={cwd}
            disabled={busy}
            onChange={(e) => setCwd(e.target.value)}
            placeholder="/Users/me/github/myproject"
            style={inputStyle()}
          />
          <Hint>{t("form.hints.cwd")}</Hint>
        </Field>
        <Field label={t("form.fields.prompt")}>
          <textarea
            value={prompt}
            disabled={busy}
            onChange={(e) => setPrompt(e.target.value)}
            rows={4}
            placeholder={t("form.placeholders.prompt")}
            style={{ ...inputStyle(), resize: "vertical", minHeight: "5rem" }}
          />
        </Field>
        <Field label={t("form.fields.systemPrompt")}>
          <textarea
            value={systemPrompt}
            disabled={busy}
            onChange={(e) => setSystemPrompt(e.target.value)}
            rows={2}
            style={{ ...inputStyle(), resize: "vertical", minHeight: "3rem" }}
          />
        </Field>
        <Field label={t("form.fields.appendSystemPrompt")}>
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
      <Group title={t("form.groups.how")}>
        <Field label={t("form.fields.model")}>
          <input
            type="text"
            value={model}
            disabled={busy}
            onChange={(e) => setModel(e.target.value)}
            placeholder="haiku | sonnet | opus | claude-sonnet-4-6"
            style={inputStyle()}
          />
        </Field>
        <Field label={t("form.fields.fallbackModel")}>
          <input
            type="text"
            value={fallbackModel}
            disabled={busy}
            onChange={(e) => setFallbackModel(e.target.value)}
            placeholder="haiku"
            style={inputStyle()}
          />
        </Field>
        <Field label={t("form.fields.permissionMode")}>
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
          <Hint>{t("form.hints.permissionMode")}</Hint>
        </Field>
        <Field label={t("form.fields.allowedTools")}>
          <input
            type="text"
            value={allowedToolsText}
            disabled={busy}
            onChange={(e) => setAllowedToolsText(e.target.value)}
            placeholder="Read Grep Glob Bash(git *)"
            spellCheck={false}
            style={{
              ...inputStyle(bypassWithoutTools),
              fontFamily: "var(--font-mono)",
            }}
          />
          {bypassWithoutTools && (
            <Hint kind="error">{t("form.errors.bypassNeedsTools")}</Hint>
          )}
        </Field>
        <Field label={t("form.fields.maxBudget")}>
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
            <Hint kind="error">{t("form.errors.budget")}</Hint>
          )}
        </Field>
        <Field label={t("form.fields.outputFormat")}>
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
        <Field label={t("form.fields.bareMode")}>
          <Toggle
            checked={bareMode}
            onChange={setBareMode}
            disabled={busy}
            label={t("form.fields.bareToggle")}
          />
        </Field>
      </Group>

      {/* Agent spec — the richer construction knobs (Phase 1) */}
      <Group title={t("form.groups.spec")}>
        <Field label={t("form.fields.disallowedTools")}>
          <input
            type="text"
            value={disallowedToolsText}
            disabled={busy}
            onChange={(e) => setDisallowedToolsText(e.target.value)}
            placeholder="WebFetch Bash(rm *)"
            spellCheck={false}
            style={{ ...inputStyle(), fontFamily: "var(--font-mono)" }}
          />
          <Hint>{t("form.hints.disallowedTools")}</Hint>
        </Field>

        <Field label={t("form.fields.mcpServers")}>
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--sp-8)",
              flexWrap: "wrap",
            }}
          >
            <Button
              variant={memoryAttached ? "subtle" : "outline"}
              onClick={toggleMemoryServer}
              disabled={busy}
            >
              {memoryAttached
                ? t("form.mcp.detach")
                : t("form.mcp.attach")}
            </Button>
            {memoryAttached && (
              <Tag tone="accent">claudepot-memory</Tag>
            )}
            {customMcpServers.map((s) => (
              <Tag key={s.name} tone="neutral" title={t("form.mcp.customTitle")}>
                {s.name}
              </Tag>
            ))}
          </div>
          <Hint>{t("form.hints.mcp")}</Hint>
        </Field>

        <Field label={t("form.fields.runAs")}>
          <input
            type="text"
            value={runAs}
            disabled={busy}
            onChange={(e) => setRunAs(e.target.value)}
            placeholder="you@example.com"
            spellCheck={false}
            style={inputStyle()}
          />
          <Hint>{t("form.hints.runAs")}</Hint>
        </Field>

        <Field label={t("form.fields.taskBudget")}>
          <input
            type="number"
            step="1"
            min="1"
            value={taskBudget}
            disabled={busy}
            onChange={(e) => setTaskBudget(e.target.value)}
            placeholder="50000"
            style={inputStyle(taskBudgetInvalid)}
          />
          {taskBudgetInvalid && (
            <Hint kind="error">{t("form.errors.taskBudget")}</Hint>
          )}
        </Field>

        <Field label={t("form.fields.rateMin")}>
          <input
            type="number"
            step="1"
            min="1"
            value={rateMinInterval}
            disabled={busy}
            onChange={(e) => setRateMinInterval(e.target.value)}
            placeholder="3600"
            style={inputStyle(minIntervalInvalid)}
          />
          {minIntervalInvalid && (
            <Hint kind="error">{t("form.errors.rateMin")}</Hint>
          )}
        </Field>
        <Field label={t("form.fields.rateMax")}>
          <input
            type="number"
            step="1"
            min="1"
            value={rateMaxPerDay}
            disabled={busy}
            onChange={(e) => setRateMaxPerDay(e.target.value)}
            placeholder="24"
            style={inputStyle(maxPerDayInvalid)}
          />
          {maxPerDayInvalid && (
            <Hint kind="error">{t("form.errors.rateMax")}</Hint>
          )}
        </Field>

        <Field label={t("form.fields.lifecycle")}>
          <div>
            {/* `lifecycle` is a wire value (`AgentSummaryDto.lifecycle`
                is typed `string`). Only the two the UI knows about get
                a display label; anything else renders raw rather than
                being mislabeled as "installed". */}
            <Tag tone={lifecycle === "installed" ? "ok" : "neutral"}>
              {lifecycle === "draft"
                ? t("form.lifecycle.draft")
                : lifecycle === "installed"
                  ? t("form.lifecycle.installed")
                  : lifecycle}
            </Tag>
          </div>
          <Hint>{t("form.hints.lifecycle")}</Hint>
        </Field>
      </Group>

      {/* When it runs */}
      <Group title={t("form.groups.when")}>
        <Field label={t("form.fields.triggerType")}>
          <select
            value={triggerKind}
            disabled={busy}
            onChange={(e) =>
              setTriggerKind(e.target.value as FormTriggerKind)
            }
            style={inputStyle()}
          >
            <option value="cron">{t("form.options.triggerCron")}</option>
            <option value="event">{t("form.options.triggerEvent")}</option>
            <option value="manual">{t("form.options.triggerManual")}</option>
          </select>
          <Hint>
            {triggerKind === "event"
              ? t("form.hints.triggerEvent")
              : triggerKind === "manual"
                ? t("form.hints.triggerManual")
                : t("form.hints.triggerCron")}
          </Hint>
        </Field>

        {triggerKind === "cron" && (
          <CronInput
            value={cron}
            onChange={setCron}
            onValidityChange={setCronValid}
            disabled={busy}
          />
        )}

        {triggerKind === "event" && (
          <Field label={t("form.fields.debounce")}>
            <input
              type="number"
              step="1"
              min="1"
              value={eventDebounceSecs}
              disabled={busy}
              onChange={(e) => setEventDebounceSecs(e.target.value)}
              placeholder={String(DEFAULT_EVENT_DEBOUNCE_SECS)}
              style={inputStyle(eventDebounceInvalid)}
              aria-invalid={eventDebounceInvalid}
            />
            {eventDebounceInvalid ? (
              <Hint kind="error">{t("form.errors.debounce")}</Hint>
            ) : (
              <Hint>
                {t("form.hints.debounceDefault", {
                  secs: DEFAULT_EVENT_DEBOUNCE_SECS,
                })}
              </Hint>
            )}
            {rateLimitMissingForEvent && (
              <Hint kind="error">
                {t("form.errors.eventNeedsRateLimit")}
              </Hint>
            )}
          </Field>
        )}
      </Group>

      {/* Platform behavior */}
      <Group
        title={t("form.groups.platform", {
          scheduler: capabilities?.native_label ?? "—",
        })}
      >
        <Toggle
          checked={platformOptions.wake_to_run}
          onChange={(v) =>
            setPlatformOptions({ ...platformOptions, wake_to_run: v })
          }
          disabled={busy || !capabilities?.wake_to_run}
          label={`${t("form.platform.wake")}${
            capabilities?.wake_to_run ? "" : t("form.platform.notSupported")
          }`}
        />
        <Toggle
          checked={platformOptions.catch_up_if_missed}
          onChange={(v) =>
            setPlatformOptions({ ...platformOptions, catch_up_if_missed: v })
          }
          disabled={busy || !capabilities?.catch_up_if_missed}
          label={`${t("form.platform.catchUp")}${
            capabilities?.catch_up_if_missed
              ? ""
              : t("form.platform.notSupported")
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
          label={`${t("form.platform.runLoggedOut")}${
            capabilities?.run_when_logged_out
              ? ""
              : t("form.platform.notSupported")
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
          {t("actions.cancel")}
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
          color: "var(--fg-muted)",
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
        color: "var(--fg-muted)",
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
        color: kind === "error" ? "var(--danger)" : "var(--fg-faint)",
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
        color: disabled ? "var(--fg-faint)" : "var(--fg-muted)",
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
