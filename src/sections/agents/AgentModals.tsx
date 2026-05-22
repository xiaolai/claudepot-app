import { useEffect, useState } from "react";
import {
  Modal,
  ModalBody,
  ModalHeader,
} from "../../components/primitives/Modal";
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
