import { useEffect, useState } from "react";
import {
  Modal,
  ModalBody,
  ModalFooter,
  ModalHeader,
} from "../../components/primitives/Modal";
import { Button } from "../../components/primitives/Button";
import { Tag } from "../../components/primitives/Tag";
import { api } from "../../api";
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
  const { pushToast } = useAppState();
  const [busy, setBusy] = useState(false);

  async function submit(dto: AgentCreateDto) {
    setBusy(true);
    try {
      const created = await api.agentsAdd(dto);
      onCreated(created);
      onClose();
    } catch (e) {
      pushToast("error", String(e));
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
        title="Add agent"
        id="add-agent-title"
        onClose={onClose}
      />
      <ModalBody>
        <AgentForm
          routes={routes}
          capabilities={capabilities}
          busy={busy}
          submitLabel="Create"
          onSubmit={submit}
          onCancel={onClose}
        />
      </ModalBody>
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
        if (!cancelled) pushToast("error", String(e));
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
      pushToast("error", String(e));
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
        title={`Edit ${target?.display_name || target?.name || ""}`.trim()}
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
            submitLabel="Save"
            onSubmit={submit}
            onCancel={onClose}
          />
        ) : (
          <div style={{ color: "var(--fg-3)", fontSize: "var(--fs-sm)" }}>
            Loading…
          </div>
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
        if (!cancelled) pushToast("error", String(e));
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
      pushToast("error", String(e));
    } finally {
      setBusy(false);
    }
  }

  if (!open || !target) return null;

  const s = details?.summary;
  const bypassFlagged = s?.permission_mode === "bypassPermissions";

  return (
    <Modal
      open={open}
      onClose={onClose}
      width="lg"
      aria-labelledby="review-agent-title"
    >
      <ModalHeader
        title={`Review & install ${target.display_name || target.name}`}
        id="review-agent-title"
        onClose={onClose}
      />
      <ModalBody>
        {!details || !s ? (
          <div style={{ color: "var(--fg-3)", fontSize: "var(--fs-sm)" }}>
            Loading…
          </div>
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
              This agent is a draft
              {details.drafted_by
                ? ` (drafted by ${details.drafted_by})`
                : ""}
              . Installing it arms the agent — Claudepot will
              materialize a scheduler artifact and the agent can run
              unattended. Review the spec below before you install.
            </p>

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
                  This agent runs with permission prompts disabled.
                  Installing it consents to an unattended, elevated
                  run scoped only by the allowed-tools whitelist
                  below. Make sure that list is tight.
                </span>
              </div>
            )}

            <ReviewGrid>
              <ReviewRow label="Prompt">
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
                <ReviewRow label="System prompt">
                  <span style={{ fontFamily: "var(--ff-mono)" }}>
                    {details.system_prompt}
                  </span>
                </ReviewRow>
              )}
              <ReviewRow label="Model">
                {s.model || "(CLI default)"}
              </ReviewRow>
              <ReviewRow label="Working dir">
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
              <ReviewRow label="Permission mode">
                {bypassFlagged ? (
                  <Tag tone="danger">{s.permission_mode}</Tag>
                ) : (
                  <span>{s.permission_mode}</span>
                )}
              </ReviewRow>
              <ReviewRow label="Allowed tools">
                {s.allowed_tools.length > 0 ? (
                  <span style={{ fontFamily: "var(--ff-mono)" }}>
                    {s.allowed_tools.join(", ")}
                  </span>
                ) : (
                  <span style={{ color: "var(--fg-3)" }}>none</span>
                )}
              </ReviewRow>
              {details.disallowed_tools.length > 0 && (
                <ReviewRow label="Disallowed tools">
                  <span style={{ fontFamily: "var(--ff-mono)" }}>
                    {details.disallowed_tools.join(", ")}
                  </span>
                </ReviewRow>
              )}
              <ReviewRow label="Trigger">
                {s.trigger_kind === "cron"
                  ? `cron ${s.cron ?? ""}${
                      s.timezone ? ` (${s.timezone})` : ""
                    }`
                  : "manual (Run-Now only)"}
              </ReviewRow>
              <ReviewRow label="Run as">
                {details.run_as || "active account at fire time"}
              </ReviewRow>
              <ReviewRow label="Task budget">
                {details.task_budget != null
                  ? `${details.task_budget} tokens/run`
                  : "no ceiling"}
              </ReviewRow>
              <ReviewRow label="Rate limit">
                {formatRateLimit(details)}
              </ReviewRow>
              {details.mcp_servers.length > 0 && (
                <ReviewRow label="MCP servers">
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
          Cancel
        </Button>
        <Button
          variant="solid"
          onClick={confirmInstall}
          disabled={busy || !details}
        >
          {busy ? "Installing…" : "Install"}
        </Button>
      </ModalFooter>
    </Modal>
  );
}

function formatRateLimit(d: AgentDetailsDto): string {
  const rl = d.rate_limit;
  if (!rl) return "none";
  const parts: string[] = [];
  if (rl.min_interval_secs != null) {
    parts.push(`min ${rl.min_interval_secs}s between runs`);
  }
  if (rl.max_per_day != null) {
    parts.push(`max ${rl.max_per_day}/day`);
  }
  return parts.length > 0 ? parts.join(", ") : "none";
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
