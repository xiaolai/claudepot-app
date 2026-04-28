// Automation CRUD + lifecycle. Sharded from src/api/.
// src/api/index.ts merges every domain slice into the canonical
// `api` object.

import { invoke } from "@tauri-apps/api/core";
import type {
  AutomationCreateDto,
  AutomationDetailsDto,
  AutomationRunDto,
  AutomationSummaryDto,
  AutomationUpdateDto,
  CronValidationDto,
  NameValidationDto,
  SchedulerCapabilitiesDto,
} from "../types";

export const automationApi = {
  automationsList: () => invoke<AutomationSummaryDto[]>("automations_list"),

  automationsGet: (id: string) =>
    invoke<AutomationDetailsDto>("automations_get", { id }),

  automationsAdd: (dto: AutomationCreateDto) =>
    invoke<AutomationSummaryDto>("automations_add", { dto }),

  automationsUpdate: (dto: AutomationUpdateDto) =>
    invoke<AutomationSummaryDto>("automations_update", { dto }),

  automationsRemove: (id: string) =>
    invoke<void>("automations_remove", { id }),

  automationsSetEnabled: (id: string, enabled: boolean) =>
    invoke<void>("automations_set_enabled", { id, enabled }),

  /** Returns op_id; subscribe to `op-progress::<op_id>`. */
  automationsRunNowStart: (id: string) =>
    invoke<string>("automations_run_now_start", { id }),

  automationsRunsList: (id: string, limit?: number) =>
    invoke<AutomationRunDto[]>("automations_runs_list", { id, limit }),

  automationsRunGet: (id: string, runId: string) =>
    invoke<AutomationRunDto>("automations_run_get", { id, runId }),

  automationsValidateName: (name: string) =>
    invoke<NameValidationDto>("automations_validate_name", { name }),

  automationsValidateCron: (expr: string) =>
    invoke<CronValidationDto>("automations_validate_cron", { expr }),

  automationsSchedulerCapabilities: () =>
    invoke<SchedulerCapabilitiesDto>("automations_scheduler_capabilities"),

  /** Returns the rendered scheduler artifact (plist / unit-files / TS XML). */
  automationsDryRunArtifact: (id: string) =>
    invoke<string>("automations_dry_run_artifact", { id }),

  automationsOpenArtifactDir: () =>
    invoke<void>("automations_open_artifact_dir"),

  automationsLingerStatus: () =>
    invoke<boolean>("automations_linger_status"),

  automationsLingerEnable: () =>
    invoke<void>("automations_linger_enable"),
};
