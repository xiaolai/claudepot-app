// Agent CRUD + lifecycle. Sharded from src/api/.
// src/api/index.ts merges every domain slice into the canonical
// `api` object.

import { invoke } from "@tauri-apps/api/core";
import type {
  AgentCreateDto,
  AgentDetailsDto,
  AgentRunDto,
  AgentSummaryDto,
  AgentUpdateDto,
  CronValidationDto,
  NameValidationDto,
  SchedulerCapabilitiesDto,
} from "../types";

export const agentApi = {
  agentsList: () => invoke<AgentSummaryDto[]>("agents_list"),

  agentsGet: (id: string) =>
    invoke<AgentDetailsDto>("agents_get", { id }),

  agentsAdd: (dto: AgentCreateDto) =>
    invoke<AgentSummaryDto>("agents_add", { dto }),

  /**
   * Arm a draft agent (draft -> installed + materialize the
   * scheduler artifact). Human-only — the CLI cannot do this.
   */
  agentInstall: (id: string) =>
    invoke<AgentSummaryDto>("agent_install", { id }),

  /**
   * Instantiate a built-in agent template as a fresh draft (F21).
   * The returned summary has `lifecycle = "draft"`; the human
   * reviews and arms it via the existing install flow. v1 ships
   * one template id: `"session-narrator"`.
   */
  agentAddFromTemplate: (templateId: string, cwd: string) =>
    invoke<AgentSummaryDto>("agent_add_from_template", {
      templateId,
      cwd,
    }),

  agentsUpdate: (dto: AgentUpdateDto) =>
    invoke<AgentSummaryDto>("agents_update", { dto }),

  agentsRemove: (id: string) =>
    invoke<void>("agents_remove", { id }),

  agentsSetEnabled: (id: string, enabled: boolean) =>
    invoke<void>("agents_set_enabled", { id, enabled }),

  /** Returns op_id; subscribe to `op-progress::<op_id>`. */
  agentsRunNowStart: (id: string) =>
    invoke<string>("agents_run_now_start", { id }),

  agentsRunsList: (id: string, limit?: number) =>
    invoke<AgentRunDto[]>("agents_runs_list", { id, limit }),

  agentsRunGet: (id: string, runId: string) =>
    invoke<AgentRunDto>("agents_run_get", { id, runId }),

  agentsValidateName: (name: string) =>
    invoke<NameValidationDto>("agents_validate_name", { name }),

  agentsValidateCron: (expr: string) =>
    invoke<CronValidationDto>("agents_validate_cron", { expr }),

  agentsSchedulerCapabilities: () =>
    invoke<SchedulerCapabilitiesDto>("agents_scheduler_capabilities"),

  /** Returns the rendered scheduler artifact (plist / unit-files / TS XML). */
  agentsDryRunArtifact: (id: string) =>
    invoke<string>("agents_dry_run_artifact", { id }),

  agentsOpenArtifactDir: () =>
    invoke<void>("agents_open_artifact_dir"),

  agentsLingerStatus: () =>
    invoke<boolean>("agents_linger_status"),

  agentsLingerEnable: () =>
    invoke<void>("agents_linger_enable"),
};
