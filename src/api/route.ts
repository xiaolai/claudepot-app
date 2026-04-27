// Route (third-party LLM backend) CRUD + lifecycle.
// Sharded from src/api.ts; src/api/index.ts merges every domain
// slice into the canonical `api` object.

import { invoke } from "@tauri-apps/api/core";
import type {
  RouteCreateDto,
  RouteSettingsDto,
  RouteSummaryDto,
  RouteUpdateDto,
} from "../types";

export const routeApi = {
  // ---------- Routes (third-party LLM backends) ----------

  /** Enumerate every defined route. Previews only — no api_key. */
  routesList: () => invoke<RouteSummaryDto[]>("routes_list"),

  routesSettingsGet: () => invoke<RouteSettingsDto>("routes_settings_get"),
  routesSettingsSet: (settings: RouteSettingsDto) =>
    invoke<RouteSettingsDto>("routes_settings_set", { settings }),

  /**
   * Define a new route. The user has just typed the API key into the
   * form — pass it in once. Rust never round-trips it back; subsequent
   * `routesList()` calls return only `api_key_preview`.
   */
  routesAdd: (route: RouteCreateDto) =>
    invoke<RouteSummaryDto>("routes_add", { route }),

  routesEdit: (route: RouteUpdateDto) =>
    invoke<RouteSummaryDto>("routes_edit", { route }),

  routesRemove: (id: string) => invoke<void>("routes_remove", { id }),

  /** Materialize the wrapper script under `~/.claudepot/bin/<name>`. */
  routesUseCli: (id: string) =>
    invoke<RouteSummaryDto>("routes_use_cli", { id }),
  routesUnuseCli: (id: string) =>
    invoke<RouteSummaryDto>("routes_unuse_cli", { id }),

  /**
   * Mirror this route's keys into Claude Desktop's
   * `enterpriseConfig` (and write a `configLibrary/<uuid>.json`
   * profile). At most one route is active at a time — calling
   * this on route B clears it on whichever was active before.
   */
  routesUseDesktop: (id: string) =>
    invoke<RouteSummaryDto>("routes_use_desktop", { id }),
  routesUnuseDesktop: () => invoke<void>("routes_unuse_desktop"),

  /** Returns `claude-<slug>` derived from the model field. */
  routesDeriveSlug: (model: string) =>
    invoke<string>("routes_derive_slug", { model }),
  /** Throws on invalid wrapper name (reserved, path-y, empty, …). */
  routesValidateWrapperName: (name: string) =>
    invoke<string>("routes_validate_wrapper_name", { name }),

  /** Best-effort renderer-side cleanup of a previously-sent secret. */
  routesZeroSecret: (secret: string) =>
    invoke<void>("routes_zero_secret", { secret }),

  /** Is Claude Desktop currently running? */
  routesDesktopRunning: () => invoke<boolean>("routes_desktop_running"),
  /** Quit and relaunch Claude Desktop to apply enterpriseConfig changes. */
  routesDesktopRestart: () => invoke<void>("routes_desktop_restart"),
};
