import { useEffect, useState } from "react";
import {
  Modal,
  ModalBody,
  ModalHeader,
} from "../../components/primitives/Modal";
import { api } from "../../api";
import type {
  AutomationCreateDto,
  AutomationDetailsDto,
  AutomationSummaryDto,
  AutomationUpdateDto,
  RouteSummaryDto,
  SchedulerCapabilitiesDto,
} from "../../types";
import { AutomationForm } from "./AutomationForm";

interface AddProps {
  open: boolean;
  routes: RouteSummaryDto[];
  capabilities: SchedulerCapabilitiesDto | null;
  onClose: () => void;
  onCreated: (a: AutomationSummaryDto) => void;
  onError: (msg: string) => void;
}

export function AddAutomationModal({
  open,
  routes,
  capabilities,
  onClose,
  onCreated,
  onError,
}: AddProps) {
  const [busy, setBusy] = useState(false);

  async function submit(dto: AutomationCreateDto) {
    setBusy(true);
    try {
      const created = await api.automationsAdd(dto);
      onCreated(created);
      onClose();
    } catch (e) {
      onError(String(e));
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
      aria-labelledby="add-automation-title"
    >
      <ModalHeader
        title="Add automation"
        id="add-automation-title"
        onClose={onClose}
      />
      <ModalBody>
        <AutomationForm
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
  target: AutomationSummaryDto | null;
  routes: RouteSummaryDto[];
  capabilities: SchedulerCapabilitiesDto | null;
  onClose: () => void;
  onUpdated: (a: AutomationSummaryDto) => void;
  onError: (msg: string) => void;
}

export function EditAutomationModal({
  open,
  target,
  routes,
  capabilities,
  onClose,
  onUpdated,
  onError,
}: EditProps) {
  const [details, setDetails] = useState<AutomationDetailsDto | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open || !target) {
      setDetails(null);
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        const d = await api.automationsGet(target.id);
        if (!cancelled) setDetails(d);
      } catch (e) {
        onError(String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open, target, onError]);

  async function submit(dto: AutomationCreateDto) {
    if (!target) return;
    setBusy(true);
    try {
      const update: AutomationUpdateDto = {
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
      };
      const updated = await api.automationsUpdate(update);
      onUpdated(updated);
      onClose();
    } catch (e) {
      onError(String(e));
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
      aria-labelledby="edit-automation-title"
    >
      <ModalHeader
        title={`Edit ${target?.display_name || target?.name || ""}`.trim()}
        id="edit-automation-title"
        onClose={onClose}
      />
      <ModalBody>
        {details ? (
          <AutomationForm
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
