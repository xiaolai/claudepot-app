// Template gallery + install. Sharded from src/api/.
// src/api/index.ts merges every domain slice into the canonical
// `api` object.

import { invoke } from "@tauri-apps/api/core";
import type {
  AutomationSummaryDto,
  TemplateDetailsDto,
  TemplateInstanceDto,
  TemplateRouteSummaryDto,
  TemplateSummaryDto,
} from "../types";

export const templateApi = {
  templatesList: () => invoke<TemplateSummaryDto[]>("templates_list"),

  templatesGet: (id: string) =>
    invoke<TemplateDetailsDto>("templates_get", { id }),

  templatesSampleReport: (id: string) =>
    invoke<string>("templates_sample_report", { id }),

  templatesCapableRoutes: (id: string) =>
    invoke<TemplateRouteSummaryDto[]>("templates_capable_routes", { id }),

  templatesInstall: (instance: TemplateInstanceDto) =>
    invoke<AutomationSummaryDto>("templates_install", { instance }),
};
