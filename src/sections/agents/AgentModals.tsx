import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Modal,
  ModalBody,
  ModalFooter,
  ModalHeader,
} from "../../components/primitives/Modal";
import { Button } from "../../components/primitives/Button";
import { Tag } from "../../components/primitives/Tag";
import { api } from "../../api";
import { renderError } from "../../lib/i18n-error";
import { i18n } from "../../lib/i18n";
import { useAppState } from "../../providers/AppStateProvider";
import type {
  AgentCreateDto,
  AgentDetailsDto,
  AgentSummaryDto,
  AgentUpdateDto,
  RouteSummaryDto,
  SchedulerCapabilitiesDto,
} from "../../types";
import { AgentForm } from "./AgentForm";
import { SkeletonRows } from "../../components/primitives/Skeleton";

interface AddProps {
  open: boolean;
  routes: RouteSummaryDto[];
  capabilities: SchedulerCapabilitiesDto | null;
  onClose: () => void;
  onCreated: (a: AgentSummaryDto) => void;
}

export function AddAgentModal({
  open,
  routes,
  capabilities,
  onClose,
  onCreated,
}: AddProps) {
  const { t } = useTranslation("agents");
  const { pushToast } = useAppState();
  const [busy, setBusy] = useState(false);

  async function submit(dto: AgentCreateDto) {
    setBusy(true);
    try {
      const created = await api.agentsAdd(dto);
      onCreated(created);
      onClose();
    } catch (e) {
      pushToast("error", renderError(e));
    } finally {
      setBusy(false);
    }
  }

  if (!open) return null;
  return (
    <Modal
      open={open}
      onClose={onClose}
      width="lg"
      aria-labelledby="add-agent-title"
    >
      <ModalHeader
        title={t("add.title")}
        id="add-agent-title"
        onClose={onClose}
      />
      <ModalBody>
        <AgentForm
          routes={routes}
          capabilities={capabilities}
          busy={busy}
          submitLabel={t("add.submit")}
          onSubmit={submit}
          onCancel={onClose}
        />
      </ModalBody>
    </Modal>
  );
}

interface AddFromBuiltinTemplateProps {
  open: boolean;
  /** v1 ships only `"session-narrator"`. */
  templateId: string;
  templateName: string;
  onClose: () => void;
  onCreated: (a: AgentSummaryDto) => void;
}

/**
 * Phase-4 (F21) affordance: instantiate a built-in Rust-side agent
 * template (e.g. Session Narrator) as a fresh **draft**. The user
 * picks the project the narrator should watch (`cwd`); the backend
 * stamps `created_via = Template` and saves it as `Lifecycle::Draft`.
 * The user then arms it via the existing Review & install flow.
 *
 * Distinct from `TemplateGallery`, which installs *blueprint*-defined
 * templates (the catalog under `core::templates`). The built-in
 * Session Narrator is the single template that lives in
 * `claudepot_core::agent::templates::session_narrator` — the headline
 * v1 reactive agent.
 */
export function AddFromBuiltinTemplateModal({
  open,
  templateId,
  templateName,
  onClose,
  onCreated,
}: AddFromBuiltinTemplateProps) {
  const { t } = useTranslation("agents");
  const { pushToast } = useAppState();
  const [cwd, setCwd] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (open) setCwd("");
  }, [open]);

  async function create() {
    if (!cwd.trim()) {
      pushToast("error", t("template.cwdRequired"));
      return;
    }
    setBusy(true);
    try {
      const created = await api.agentAddFromTemplate(
        templateId,
        cwd.trim(),
      );
      onCreated(created);
      onClose();
    } catch (e) {
      pushToast("error", renderError(e));
    } finally {
      setBusy(false);
    }
  }

  if (!open) return null;
  return (
    <Modal
      open={open}
      onClose={onClose}
      width="md"
      aria-labelledby="add-builtin-template-title"
    >
      <ModalHeader
        title={t("template.title", { name: templateName })}
        id="add-builtin-template-title"
        onClose={onClose}
      />
      <ModalBody>
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: "var(--sp-12)",
          }}
        >
          <p
            style={{
              margin: 0,
              fontSize: "var(--fs-sm)",
              color: "var(--fg-2)",
            }}
          >
            {t("template.body", { name: templateName })}
          </p>
          <label
            style={{
              display: "flex",
              flexDirection: "column",
              gap: "var(--sp-4)",
              fontSize: "var(--fs-sm)",
              color: "var(--fg)",
            }}
          >
            <span>{t("template.cwdLabel")}</span>
            <input
              type="text"
              value={cwd}
              onChange={(e) => setCwd(e.target.value)}
              placeholder="/Users/me/projects/example"
              spellCheck={false}
              autoFocus
              style={{
                fontFamily: "var(--ff-mono)",
                fontSize: "var(--fs-sm)",
                padding: "var(--sp-6) var(--sp-8)",
                background: "var(--bg-raised)",
                color: "var(--fg)",
                border: "var(--bw-hair) solid var(--line)",
                borderRadius: "var(--r-2)",
              }}
            />
          </label>
        </div>
      </ModalBody>
      <ModalFooter>
        <Button variant="ghost" onClick={onClose} disabled={busy}>
          {t("actions.cancel")}
        </Button>
        <Button
          variant="solid"
          onClick={create}
          disabled={busy || !cwd.trim()}
        >
          {busy ? t("template.creating") : t("template.submit")}
        </Button>
      </ModalFooter>
    </Modal>
  );
}

interface EditProps {
  open: boolean;
  target: AgentSummaryDto | null;
  routes: RouteSummaryDto[];
  capabilities: SchedulerCapabilitiesDto | null;
  onClose: () => void;
  onUpdated: (a: AgentSummaryDto) => void;
}

export function EditAgentModal({
  open,
  target,
  routes,
  capabilities,
  onClose,
  onUpdated,
}: EditProps) {
  const { t } = useTranslation("agents");
  const { pushToast } = useAppState();
  const [details, setDetails] = useState<AgentDetailsDto | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open || !target) {
      setDetails(null);
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        const d = await api.agentsGet(target.id);
        if (!cancelled) setDetails(d);
      } catch (e) {
        // Guard the toast too: if the modal closed (or the target
        // switched) while the fetch was in flight, a late rejection
        // must not fire a stray error toast.
        if (!cancelled) pushToast("error", renderError(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open, target, pushToast]);

  async function submit(dto: AgentCreateDto) {
    if (!target) return;
    setBusy(true);
    try {
      const update: AgentUpdateDto = {
        id: target.id,
        display_name: dto.display_name,
        description: dto.description,
        model: dto.model,
        cwd: dto.cwd,
        prompt: dto.prompt,
        system_prompt: dto.system_prompt,
        append_system_prompt: dto.append_system_prompt,
        permission_mode: dto.permission_mode,
        allowed_tools: dto.allowed_tools,
        add_dir: dto.add_dir,
        max_budget_usd: dto.max_budget_usd,
        fallback_model: dto.fallback_model,
        output_format: dto.output_format,
        json_schema: dto.json_schema,
        bare: dto.bare,
        extra_env: dto.extra_env,
        cron: dto.cron,
        timezone: dto.timezone,
        platform_options: dto.platform_options,
        log_retention_runs: dto.log_retention_runs,
        // Phase-1 spec fields.
        disallowed_tools: dto.disallowed_tools,
        mcp_servers: dto.mcp_servers,
        run_as: dto.run_as,
        // 0 = "clear the budget" on the update path; the form sends
        // 0 when the field is empty.
        task_budget: dto.task_budget ?? 0,
        rate_limit: dto.rate_limit,
      };
      const updated = await api.agentsUpdate(update);
      onUpdated(updated);
      onClose();
    } catch (e) {
      pushToast("error", renderError(e));
    } finally {
      setBusy(false);
    }
  }

  if (!open) return null;
  return (
    <Modal
      open={open}
      onClose={onClose}
      width="lg"
      aria-labelledby="edit-agent-title"
    >
      <ModalHeader
        title={t("edit.title", {
          name: target?.display_name || target?.name || "",
        }).trim()}
        id="edit-agent-title"
        onClose={onClose}
      />
      <ModalBody>
        {details ? (
          <AgentForm
            initial={details}
            routes={routes}
            capabilities={capabilities}
            busy={busy}
            submitLabel={t("edit.submit")}
            onSubmit={submit}
            onCancel={onClose}
          />
        ) : (
          // Was a one-line `Loading…` inside a modal body — the dialog resized as the form landed.
          <SkeletonRows rows={4} label={t("loading")} />
        )}
      </ModalBody>
    </Modal>
  );
}

interface ReviewProps {
  open: boolean;
  target: AgentSummaryDto | null;
  onClose: () => void;
  onInstalled: (a: AgentSummaryDto) => void;
}

/**
 * Review-and-install modal for a draft agent — the human-approval
 * gate (PRD §8.3). A draft (likely AI-authored) is inert; this
 * modal shows the full spec the human is consenting to before
 * arming it, then calls `agent_install`.
 *
 * `bypassPermissions` is visually flagged: arming a
 * bypassPermissions agent means consenting to an unattended,
 * elevated `claude -p` run, so the human must see it clearly.
 */
export function ReviewInstallModal({
  open,
  target,
  onClose,
  onInstalled,
}: ReviewProps) {
  const { t } = useTranslation("agents");
  const { pushToast } = useAppState();
  const [details, setDetails] = useState<AgentDetailsDto | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open || !target) {
      setDetails(null);
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        const d = await api.agentsGet(target.id);
        if (!cancelled) setDetails(d);
      } catch (e) {
        if (!cancelled) pushToast("error", renderError(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open, target, pushToast]);

  async function confirmInstall() {
    if (!target) return;
    setBusy(true);
    try {
      const installed = await api.agentInstall(target.id);
      onInstalled(installed);
      onClose();
    } catch (e) {
      pushToast("error", renderError(e));
    } finally {
      setBusy(false);
    }
  }

  if (!open || !target) return null;

  const s = details?.summary;
  const bypassFlagged = s?.permission_mode === "bypassPermissions";
  // F19: `created_via` is the trustworthy "this was not
  // user-authored" signal. The free-text `drafted_by` is advisory;
  // an AI client can set `--drafted-by "manual-setup"` to launder
  // a draft. `created_via` is stamped by the code path itself.
  const createdVia = details?.created_via ?? "gui";
  const notHandAuthored = createdVia !== "gui";
  // F22: `run_as` is recorded but the per-run credential injection
  // is not yet wired (`shim.rs`). Surface a clear warning before
  // the human arms the agent so they know the saved value will not
  // take effect at fire time.
  const runAsFlagged = !!details?.run_as && details.run_as.length > 0;

  return (
    <Modal
      open={open}
      onClose={onClose}
      width="lg"
      aria-labelledby="review-agent-title"
    >
      <ModalHeader
        title={t("review.title", {
          name: target.display_name || target.name,
        })}
        id="review-agent-title"
        onClose={onClose}
      />
      <ModalBody>
        {!details || !s ? (
          // Was a one-line `Loading…` inside a modal body — the dialog resized as the form landed.
          <SkeletonRows rows={4} label={t("loading")} />
        ) : (
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              gap: "var(--sp-12)",
            }}
          >
            <p
              style={{
                margin: 0,
                fontSize: "var(--fs-sm)",
                color: "var(--fg-2)",
              }}
            >
              {/* Two whole sentences, not a stitched fragment: the
                  "(drafted by …)" aside sits mid-sentence, and a
                  translator must be free to move it. This copy is the
                  arming gate — it has to keep saying that a draft does
                  not run until this install. */}
              {details.drafted_by
                ? t("review.introDrafted", { name: details.drafted_by })
                : t("review.intro")}
            </p>

            {notHandAuthored && (
              <div
                role="alert"
                aria-label={t("review.origin.aria")}
                style={{
                  display: "flex",
                  flexDirection: "column",
                  gap: "var(--sp-4)",
                  padding: "var(--sp-8) var(--sp-12)",
                  border: "var(--bw-hair) solid var(--accent)",
                  borderRadius: "var(--r-2)",
                  background: "var(--bg)",
                  fontSize: "var(--fs-sm)",
                  color: "var(--fg-2)",
                }}
              >
                <div
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: "var(--sp-6)",
                  }}
                >
                  <Tag tone="accent">
                    {createdVia === "cli_draft"
                      ? t("review.origin.aiDrafted")
                      : t("review.origin.templateDrafted")}
                  </Tag>
                  <span style={{ color: "var(--fg)" }}>
                    {t("review.origin.notGui")}
                  </span>
                </div>
                <span>
                  {createdVia === "cli_draft"
                    ? t("review.origin.bodyCli")
                    : t("review.origin.bodyTemplate")}
                </span>
              </div>
            )}

            {bypassFlagged && (
              <div
                role="alert"
                style={{
                  display: "flex",
                  alignItems: "flex-start",
                  gap: "var(--sp-8)",
                  padding: "var(--sp-8) var(--sp-12)",
                  border: "var(--bw-hair) solid var(--danger)",
                  borderRadius: "var(--r-2)",
                  background: "var(--bg)",
                  fontSize: "var(--fs-sm)",
                  color: "var(--danger)",
                }}
              >
                <Tag tone="danger">bypassPermissions</Tag>
                <span style={{ color: "var(--fg-2)" }}>
                  {t("review.bypassBody")}
                </span>
              </div>
            )}

            <ReviewGrid>
              <ReviewRow label={t("review.rows.prompt")}>
                <pre
                  style={{
                    margin: 0,
                    whiteSpace: "pre-wrap",
                    fontFamily: "var(--ff-mono)",
                    fontSize: "var(--fs-xs)",
                    color: "var(--fg)",
                  }}
                >
                  {details.prompt}
                </pre>
              </ReviewRow>
              {details.system_prompt && (
                <ReviewRow label={t("review.rows.systemPrompt")}>
                  <span style={{ fontFamily: "var(--ff-mono)" }}>
                    {details.system_prompt}
                  </span>
                </ReviewRow>
              )}
              <ReviewRow label={t("review.rows.model")}>
                {s.model || t("review.values.cliDefault")}
              </ReviewRow>
              <ReviewRow label={t("review.rows.workingDir")}>
                <span
                  className="selectable"
                  style={{
                    fontFamily: "var(--ff-mono)",
                    userSelect: "text",
                  }}
                >
                  {s.cwd}
                </span>
              </ReviewRow>
              <ReviewRow label={t("review.rows.permissionMode")}>
                {bypassFlagged ? (
                  <Tag tone="danger">{s.permission_mode}</Tag>
                ) : (
                  <span>{s.permission_mode}</span>
                )}
              </ReviewRow>
              <ReviewRow label={t("review.rows.allowedTools")}>
                {s.allowed_tools.length > 0 ? (
                  <span style={{ fontFamily: "var(--ff-mono)" }}>
                    {s.allowed_tools.join(", ")}
                  </span>
                ) : (
                  <span style={{ color: "var(--fg-3)" }}>
                    {t("review.values.none")}
                  </span>
                )}
              </ReviewRow>
              {details.disallowed_tools.length > 0 && (
                <ReviewRow label={t("review.rows.disallowedTools")}>
                  <span style={{ fontFamily: "var(--ff-mono)" }}>
                    {details.disallowed_tools.join(", ")}
                  </span>
                </ReviewRow>
              )}
              <ReviewRow label={t("review.rows.trigger")}>
                {/* The cron expression and its IANA timezone are data
                    and pass through verbatim; only the surrounding
                    label is translated. */}
                {s.trigger_kind === "cron"
                  ? t("review.values.triggerCron", {
                      value: `${s.cron ?? ""}${
                        s.timezone ? ` (${s.timezone})` : ""
                      }`,
                    })
                  : t("review.values.triggerManual")}
              </ReviewRow>
              <ReviewRow label={t("review.rows.runAs")}>
                {runAsFlagged ? (
                  <div
                    style={{
                      display: "flex",
                      flexDirection: "column",
                      gap: "var(--sp-4)",
                    }}
                  >
                    <span style={{ fontFamily: "var(--ff-mono)" }}>
                      {details.run_as}
                    </span>
                    <span
                      role="note"
                      style={{
                        color: "var(--accent-ink)",
                        fontSize: "var(--fs-xs)",
                      }}
                    >
                      {t("review.values.runAsWarning")}
                    </span>
                  </div>
                ) : (
                  <span>{t("review.values.activeAccount")}</span>
                )}
              </ReviewRow>
              <ReviewRow label={t("review.rows.taskBudget")}>
                {details.task_budget != null
                  ? t("review.values.tokensPerRun", {
                      tokens: details.task_budget,
                    })
                  : t("review.values.noCeiling")}
              </ReviewRow>
              <ReviewRow label={t("review.rows.rateLimit")}>
                {formatRateLimit(details)}
              </ReviewRow>
              {details.mcp_servers.length > 0 && (
                <ReviewRow label={t("review.rows.mcpServers")}>
                  {details.mcp_servers
                    .map((m) =>
                      m.kind === "claudepot_memory"
                        ? "claudepot-memory"
                        : m.name,
                    )
                    .join(", ")}
                </ReviewRow>
              )}
            </ReviewGrid>
          </div>
        )}
      </ModalBody>
      <ModalFooter>
        <Button variant="ghost" onClick={onClose} disabled={busy}>
          {t("actions.cancel")}
        </Button>
        <Button
          variant="solid"
          onClick={confirmInstall}
          disabled={busy || !details}
        >
          {busy ? t("review.installing") : t("review.install")}
        </Button>
      </ModalFooter>
    </Modal>
  );
}

/**
 * Plain helper, not a component — it resolves through the module
 * `i18n` per the non-component convention. It is called during
 * ReviewInstallModal's render, whose `useTranslation` subscription is
 * what re-runs it on a language switch.
 */
function formatRateLimit(d: AgentDetailsDto): string {
  const none = i18n.t("review.values.rateNone", { ns: "agents" });
  const rl = d.rate_limit;
  if (!rl) return none;
  const parts: string[] = [];
  if (rl.min_interval_secs != null) {
    parts.push(
      i18n.t("review.values.rateMin", {
        ns: "agents",
        secs: rl.min_interval_secs,
      }),
    );
  }
  if (rl.max_per_day != null) {
    parts.push(
      i18n.t("review.values.rateMax", {
        ns: "agents",
        perDay: rl.max_per_day,
      }),
    );
  }
  return parts.length > 0 ? parts.join(", ") : none;
}

function ReviewGrid({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "max-content 1fr",
        gap: "var(--sp-8) var(--sp-16)",
        fontSize: "var(--fs-sm)",
        color: "var(--fg)",
      }}
    >
      {children}
    </div>
  );
}

function ReviewRow({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <>
      <span
        style={{
          color: "var(--fg-3)",
          fontSize: "var(--fs-xs)",
          paddingTop: "var(--sp-2)",
        }}
      >
        {label}
      </span>
      <div style={{ minWidth: 0 }}>{children}</div>
    </>
  );
}
