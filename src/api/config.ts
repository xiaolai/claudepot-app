// Config view: scan, preview, search, effective settings, MCP, editors, watch.
// Sharded from src/api.ts; src/api/index.ts merges every
// domain slice into the canonical `api` object.

import { invoke } from "@tauri-apps/api/core";
import type {
  ConfigTreeDto,
  ConfigPreviewDto,
  ConfigKind,
  ConfigEffectiveSettingsDto,
  ConfigEffectiveMcpDto,
  EditorCandidateDto,
  EditorDefaultsDto,
  McpSimulationMode,
} from "../types";

export const configApi = {
  // Config section — P0 surface.
  configScan: (cwd?: string | null) =>
    invoke<ConfigTreeDto>("config_scan", { cwd: cwd ?? null }),
  configPreview: (nodeId: string) =>
    invoke<ConfigPreviewDto>("config_preview", { nodeId }),
  configSearchStart: (
    searchId: string,
    query: {
      text: string;
      regex?: boolean;
      case_sensitive?: boolean;
      scope_filter?: string[] | null;
    },
  ) => invoke<void>("config_search_start", { searchId, query }),
  configSearchCancel: (searchId: string) =>
    invoke<void>("config_search_cancel", { searchId }),
  configEffectiveSettings: (cwd?: string | null) =>
    invoke<ConfigEffectiveSettingsDto>("config_effective_settings", {
      cwd: cwd ?? null,
    }),
  configEffectiveMcp: (mode: McpSimulationMode, cwd?: string | null) =>
    invoke<ConfigEffectiveMcpDto>("config_effective_mcp", {
      cwd: cwd ?? null,
      mode,
    }),
  configListEditors: (force?: boolean) =>
    invoke<EditorCandidateDto[]>("config_list_editors", { force: !!force }),
  configGetEditorDefaults: () =>
    invoke<EditorDefaultsDto>("config_get_editor_defaults"),
  configSetEditorDefault: (kind: ConfigKind | null, editorId: string) =>
    invoke<void>("config_set_editor_default", { kind, editorId }),
  configOpenInEditorPath: (
    path: string,
    editorId: string | null,
    kindHint: ConfigKind | null,
  ) =>
    invoke<void>("config_open_in_editor_path", {
      path,
      editorId,
      kindHint,
    }),
  configWatchStart: (cwd?: string | null) =>
    invoke<void>("config_watch_start", { cwd: cwd ?? null }),
  configWatchStop: () => invoke<void>("config_watch_stop"),

};
