import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { useVirtualizer } from "@tanstack/react-virtual";
import { api } from "../api";
import type {
  ConfigAnchor,
  ConfigFileNodeDto,
  ConfigKind,
  ConfigPreviewDto,
  ConfigSearchHitDto,
  ConfigSearchSummaryDto,
  ConfigTreeDto,
  EditorCandidateDto,
  EditorDefaultsDto,
  ProjectInfo,
} from "../types";
import { ScreenHeader } from "../shell/ScreenHeader";
import { PreviewHeader } from "../components/primitives/PreviewHeader";
import { Button } from "../components/primitives/Button";
import { IconButton } from "../components/primitives/IconButton";
import { BackAffordance } from "../components/primitives/BackAffordance";
import { FilterChip } from "../components/primitives/FilterChip";
import { Input } from "../components/primitives/Input";
import { Glyph } from "../components/primitives/Glyph";
import { NF, type NfIcon } from "../icons";
import { EffectiveRenderer } from "./config/EffectiveRenderer";
import { EffectiveMcpRenderer } from "./config/EffectiveMcpRenderer";
import { MarkdownRenderer } from "./config/MarkdownRenderer";
import { JsonTreeRenderer } from "./config/JsonTreeRenderer";
import { CodeRenderer } from "./config/CodeRenderer";
import { HooksRenderer, countHooksInMergedSettings } from "./config/HooksRenderer";
import { useConfigTree } from "../hooks/useConfigTree";
import { useAppState } from "../providers/AppStateProvider";

import {
  CONFIG_ANCHOR_STORAGE_KEY,
  EFFECTIVE_MCP_ROUTE,
  EFFECTIVE_SETTINGS_ROUTE,
} from "./config/constants";

const MARKDOWN_KINDS: readonly ConfigKind[] = [
  "claude_md",
  "agent",
  "skill",
  "command",
  "output_style",
  "workflow",
  "rule",
  "memory",
  "memory_index",
] as const;

const JSON_KINDS: readonly ConfigKind[] = [
  "settings",
  "settings_local",
  "managed_settings",
  "mcp_json",
  "managed_mcp_json",
  "keybindings",
  "plugin",
  "redacted_user_config",
] as const;

// Kinds the user authors. Bucketed under their own kind-group at the
// top of the tree. Order mirrors how often CC users reach for each.
// `hook` is NOT here — hooks aren't stand-alone files, they live
// inside settings.json and surface via the dedicated Hooks entry.
const DEFINITION_KINDS: readonly ConfigKind[] = [
  "agent",
  "skill",
  "command",
  "output_style",
  "workflow",
  "rule",
  "keybindings",
  "statusline",
] as const;

const DEFINITION_KIND_LABEL: Record<string, string> = {
  agent: "Agents",
  skill: "Skills",
  command: "Commands",
  output_style: "Output styles",
  workflow: "Workflows",
  rule: "Rules",
  keybindings: "Keybindings",
  statusline: "Statusline",
};

const PLUGINS_GROUP_ID = "grp:plugins";
const FILES_GROUP_ID = "grp:files";
const HOOKS_ROUTE = "virtual:hooks";

interface ConfigSectionProps {
  subRoute: string | null;
  onSubRouteChange: (subRoute: string | null) => void;
  /**
   * When provided, the section is pinned to this anchor: the picker
   * is hidden, localStorage is not consulted for the initial value,
   * and the header subtitle hides the anchor label (the surrounding
   * shell already shows which project you're in).
   *
   * Embedders:
   *   - Project shell passes `{ kind: "folder", path: <project cwd> }`
   *   - Global section passes `{ kind: "global" }`
   *
   * Standalone use (legacy top-level Config tab, if re-enabled) omits
   * this prop → picker + localStorage behavior as before.
   */
  forcedAnchor?: ConfigAnchor;
}

/**
 * Config section — read-only browser over CC's filesystem artifacts.
 *
 * Grid: two columns, both with `minWidth: 0` so long file paths and
 * long editor labels can't push the right column beyond the viewport.
 * Tree pane width is `--config-tree-width` (responsive clamp). Both
 * panes share a single virtualized tree via `@tanstack/react-virtual`
 * so the User scope's 50+ agents don't render as 50+ DOM nodes.
 *
 * `subRoute` format: `node:<id>` where `<id>` is either a FileNode.id
 * or a `virtual:*` route (Effective settings / Effective MCP). Stale
 * concrete ids are cleared after rescans; virtual ids always remain
 * valid.
 */
function loadAnchor(): ConfigAnchor {
  try {
    const raw = localStorage.getItem(CONFIG_ANCHOR_STORAGE_KEY);
    if (!raw) return { kind: "global" };
    const parsed = JSON.parse(raw);
    if (parsed?.kind === "folder" && typeof parsed.path === "string") {
      return { kind: "folder", path: parsed.path };
    }
    if (parsed?.kind === "global") return { kind: "global" };
  } catch {
    // fall through
  }
  return { kind: "global" };
}

function persistAnchor(anchor: ConfigAnchor): void {
  try {
    localStorage.setItem(CONFIG_ANCHOR_STORAGE_KEY, JSON.stringify(anchor));
  } catch {
    // localStorage may be disabled — the anchor just won't persist.
  }
}

function anchorCwd(anchor: ConfigAnchor): string | null {
  return anchor.kind === "folder" ? anchor.path : null;
}

function anchorLabel(anchor: ConfigAnchor): string {
  if (anchor.kind === "global") return "Global only";
  const p = anchor.path;
  const m = p.match(/([^/\\]+)[/\\]?$/);
  return m ? m[1] : p;
}

/**
 * Project / scope label rendered next to "Effective MCP" / "Effective
 * settings" / "Hooks" titles. Lets the user see *which* scope the
 * computed view applies to without scanning the surrounding chrome —
 * useful when ConfigSection is nested inside a project tab and the
 * outer breadcrumb is far away.
 */
function effectiveScopeLabel(anchor: ConfigAnchor): string | null {
  if (anchor.kind === "global") return "Global";
  return anchorLabel(anchor);
}

/**
 * Compact status strip for embedded ConfigSection — a slim row with
 * artifact count + dirty indicator on the left, Refresh on the right.
 * Replaces the full ScreenHeader when the surrounding shell already
 * titles the page (project tabs / Global section).
 */
function EmbeddedStatusStrip({
  artifactCount,
  updating,
  onRefresh,
}: {
  artifactCount: number | null;
  updating: boolean;
  onRefresh: () => void;
}) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        padding: "var(--sp-8) var(--sp-16)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        fontSize: "var(--fs-xs)",
        color: "var(--fg-muted)",
        flexShrink: 0,
      }}
    >
      <span>
        {artifactCount == null
          ? "Scanning…"
          : `${artifactCount} artifact${artifactCount === 1 ? "" : "s"}`}
        {updating ? " · updating…" : ""}
      </span>
      <button
        type="button"
        className="pm-focus"
        onClick={onRefresh}
        title="Re-scan on-disk artifacts"
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-4)",
          padding: "var(--sp-2) var(--sp-8)",
          background: "transparent",
          border: "none",
          color: "var(--fg-muted)",
          fontSize: "var(--fs-xs)",
          cursor: "pointer",
          borderRadius: "var(--r-sm)",
        }}
      >
        <span>Refresh</span>
      </button>
    </div>
  );
}

export function ConfigSection({
  subRoute,
  onSubRouteChange,
  forcedAnchor,
}: ConfigSectionProps) {
  const {
    tree,
    dirty: watcherDirty,
    setTree,
    orphanPatchSignal,
  } = useConfigTree(null);
  const { pushToast } = useAppState();
  const [loadError, setLoadError] = useState<string | null>(null);
  const [editors, setEditors] = useState<EditorCandidateDto[] | null>(null);
  const [defaults, setDefaults] = useState<EditorDefaultsDto | null>(null);
  const [preview, setPreview] = useState<ConfigPreviewDto | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  // Embedders win: if `forcedAnchor` is set, ignore localStorage and
  // lock the anchor. Picker, persistence, and chooseAnchor are no-ops
  // in that mode.
  const [anchor, setAnchor] = useState<ConfigAnchor>(
    () => forcedAnchor ?? loadAnchor(),
  );
  // Track identity changes of `forcedAnchor` so embedders can switch
  // between projects without unmounting the section. The JSON-stable
  // key avoids re-firing for referentially-new but value-equal props.
  const forcedAnchorKey = forcedAnchor ? JSON.stringify(forcedAnchor) : null;
  useEffect(() => {
    if (!forcedAnchor) return;
    setAnchor(forcedAnchor);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [forcedAnchorKey]);
  const [recentProjects, setRecentProjects] = useState<ProjectInfo[] | null>(null);
  // Count of hooks registered in the merged effective settings. `null`
  // while the effective-settings call is in flight or in global-only
  // mode (where it's never requested). Drives whether the Hooks row
  // is rendered in the tree — we don't show an empty placeholder.
  const [hooksCount, setHooksCount] = useState<number | null>(null);

  const selectedId = useMemo(() => {
    if (!subRoute?.startsWith("node:")) return null;
    return subRoute.slice("node:".length);
  }, [subRoute]);

  const virtualRoute = useMemo<
    null | "effective-settings" | "effective-mcp" | "hooks"
  >(() => {
    if (selectedId === EFFECTIVE_SETTINGS_ROUTE) return "effective-settings";
    if (selectedId === EFFECTIVE_MCP_ROUTE) return "effective-mcp";
    if (selectedId === HOOKS_ROUTE) return "hooks";
    return null;
  }, [selectedId]);

  const [searchQuery, setSearchQuery] = useState<string>("");
  const [searchRegex, setSearchRegex] = useState<boolean>(false);
  const [searchActive, setSearchActive] = useState<boolean>(false);
  const [activeSearchId, setActiveSearchId] = useState<string | null>(null);
  const [searchHits, setSearchHits] = useState<ConfigSearchHitDto[]>([]);
  const [searchSummary, setSearchSummary] =
    useState<ConfigSearchSummaryDto | null>(null);
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  // Holds the unlisten callbacks for the currently active search.
  // Listeners are attached BEFORE `configSearchStart()` is invoked so
  // fast searches can't drop early `hit`/`done` events that would
  // otherwise fire before a listener-effect could subscribe (audit
  // 2026-04-24, T3 H1).
  const searchUnlistenersRef = useRef<Array<() => void>>([]);

  // ⌘F focuses the content-search input. Esc clears it. Respects the
  // same modal / input gate as useSection so shortcuts never fire
  // over a modal or while typing in a text field.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.shiftKey || e.altKey) return;
      if (e.key !== "f") return;
      if (document.querySelector('[role="dialog"]')) return;
      const el = document.activeElement as HTMLElement | null;
      if (el === searchInputRef.current) return; // already there
      e.preventDefault();
      searchInputRef.current?.focus();
      searchInputRef.current?.select();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Generation token for `configScan` requests. Each call captures the
  // current gen; on return we only commit if it's still the latest.
  // Fixes the stale-write race where two anchor changes in flight could
  // land in reverse order and leave the tree showing the older anchor
  // (audit 2026-04-24, H2).
  const scanGenRef = useRef(0);

  const refreshTree = useCallback(async () => {
    const myGen = ++scanGenRef.current;
    try {
      const t = await api.configScan(anchorCwd(anchor));
      if (myGen !== scanGenRef.current) return; // superseded
      setTree(t);
      setLoadError(null);
    } catch (e) {
      if (myGen !== scanGenRef.current) return; // superseded
      setLoadError(String(e));
    }
  }, [setTree, anchor]);

  const refreshEditors = useCallback(() => {
    setEditors(null);
    void api
      .configListEditors(true)
      .then(setEditors)
      .catch(() => setEditors([]));
  }, []);

  // One-shot mount effect — editor list + defaults only. Scan + watcher
  // live in the anchor-scoped effect below, so swapping anchors
  // reliably restarts both.
  useEffect(() => {
    void api
      .configListEditors(false)
      .then(setEditors)
      .catch(() => setEditors([]));
    void api
      .configGetEditorDefaults()
      .then(setDefaults)
      .catch(() =>
        setDefaults({ by_kind: {}, fallback: "system" }),
      );
  }, []);

  // Scan + watcher are rebound whenever the anchor changes. We do NOT
  // issue a `configWatchStop` from this effect's cleanup: `configWatchStart`
  // already stops the previous watcher inside a state mutex, so firing
  // an extra `stop` on cleanup can race and kill the freshly-started
  // watcher when it arrives late (audit 2026-04-24, H1). The terminal
  // `configWatchStop` for section unmount lives in its own effect below.
  useEffect(() => {
    void refreshTree();
    void api.configWatchStart(anchorCwd(anchor)).catch(() => {
      // Non-fatal — the tree still works via explicit Refresh.
    });
  }, [anchor, refreshTree]);

  // Section-unmount only: stop the watcher so it doesn't keep emitting
  // to a webview that no longer listens. Deliberately empty deps —
  // anchor changes are handled by the effect above via the backend's
  // internal restart.
  useEffect(() => {
    return () => {
      void api.configWatchStop().catch(() => {});
    };
  }, []);

  // Hook count lives downstream of effective settings. It's anchored
  // to the same cwd as the tree, so rebind whenever anchor changes.
  // Global-only mode short-circuits to 0 — no project, no merged hooks.
  useEffect(() => {
    if (anchor.kind === "global") {
      setHooksCount(0);
      return;
    }
    let cancelled = false;
    void api
      .configEffectiveSettings(anchor.path)
      .then((r) => {
        if (cancelled) return;
        setHooksCount(countHooksInMergedSettings(r.merged));
      })
      .catch(() => {
        if (!cancelled) setHooksCount(null);
      });
    return () => {
      cancelled = true;
    };
  }, [anchor]);

  // Load the recent-projects list used by the anchor picker dropdown.
  // Cheap call — just a directory scan of `~/.claudepot/projects/`.
  useEffect(() => {
    void api
      .projectList()
      .then(setRecentProjects)
      .catch(() => setRecentProjects([]));
  }, []);

  // Recovery: orphan watcher patches without a baseline trigger a
  // fresh scan so subsequent patches have a tree to apply to.
  useEffect(() => {
    if (orphanPatchSignal === 0) return;
    void refreshTree();
  }, [orphanPatchSignal, refreshTree]);

  const chooseAnchor = useCallback(
    (next: ConfigAnchor) => {
      // Clear any virtual route selection — Effective panes are hidden
      // in global mode, so leaving the route pinned would render a
      // dead pane after the swap. Concrete node ids are checked by
      // the stale-subroute effect once the next tree arrives.
      if (next.kind === "global" && subRoute?.startsWith("node:virtual:")) {
        onSubRouteChange(null);
      }
      persistAnchor(next);
      setAnchor(next);
    },
    [subRoute, onSubRouteChange],
  );

  const pickFolderAnchor = useCallback(async () => {
    try {
      const picked = await openDialog({
        multiple: false,
        directory: true,
        title: "Choose project folder",
      });
      if (typeof picked !== "string" || picked.length === 0) return;
      chooseAnchor({ kind: "folder", path: picked });
    } catch {
      pushToast("error", "Could not open folder picker");
    }
  }, [chooseAnchor, pushToast]);

  // Repair stale subRoute. Virtual routes are always valid.
  useEffect(() => {
    if (!tree || !selectedId) return;
    if (selectedId.startsWith("virtual:")) return;
    const found = tree.scopes.some((s) =>
      s.files.some((f) => f.id === selectedId),
    );
    if (!found) onSubRouteChange(null);
  }, [tree, selectedId, onSubRouteChange]);

  // Pull preview on selection change. Virtual routes skip — their
  // renderers fetch their own data.
  //
  // Gated on `treeReady` because the backend's preview command rejects
  // with "tree not scanned yet" when `ConfigScanService` hasn't
  // committed its first tree. That happens whenever a persisted
  // `subRoute` deserializes into a `selectedId` at mount, racing the
  // initial `refreshTree()` call. Once the scan commits, the effect
  // re-fires and the preview goes through. Capturing only the
  // truthiness (not the tree reference) avoids re-fetching on every
  // watcher-driven re-scan — those don't change which file is loaded.
  const treeReady = tree !== null;
  useEffect(() => {
    if (!selectedId || selectedId.startsWith("virtual:")) {
      setPreview(null);
      setPreviewError(null);
      return;
    }
    if (!treeReady) {
      // Don't render a stale preview from a prior tree while we wait.
      setPreview(null);
      setPreviewError(null);
      return;
    }
    let cancelled = false;
    void api
      .configPreview(selectedId)
      .then((p) => {
        if (!cancelled) {
          setPreview(p);
          setPreviewError(null);
        }
      })
      .catch((e) => {
        if (!cancelled) {
          setPreview(null);
          setPreviewError(String(e));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [selectedId, treeReady]);

  // Tear down listeners registered for a previous search. Safe to call
  // when the ref is empty.
  const detachSearchListeners = useCallback(() => {
    const fns = searchUnlistenersRef.current;
    searchUnlistenersRef.current = [];
    for (const fn of fns) {
      try {
        fn();
      } catch {
        // ignore — best-effort teardown
      }
    }
  }, []);

  const startSearch = useCallback(async () => {
    const trimmed = searchQuery.trim();
    if (!trimmed) {
      detachSearchListeners();
      setSearchActive(false);
      setActiveSearchId(null);
      setSearchHits([]);
      setSearchSummary(null);
      return;
    }
    // Await cancellation of any in-flight search before starting a
    // new one — prevents the backend from briefly scoring hits on the
    // old search after it's been logically replaced.
    if (activeSearchId) {
      try {
        await api.configSearchCancel(activeSearchId);
      } catch {
        // ignore
      }
    }
    // Drop listeners from the previous search before we mint a new id.
    detachSearchListeners();
    const id = `search-${Date.now()}`;
    setSearchHits([]);
    setSearchSummary(null);
    // Attach listeners BEFORE invoking `configSearchStart`. The backend
    // begins emitting `hit`/`done` events as soon as the start command
    // returns, and fast searches (e.g. "no matches" on a tiny tree)
    // can fire `done` before a useEffect-driven subscription would
    // ever attach. Awaiting the `listen()` promises here makes the
    // ordering explicit (audit 2026-04-24, T3 H1).
    try {
      const u1 = await listen<ConfigSearchHitDto>(
        `config-search-hit::${id}`,
        (ev) => {
          setSearchHits((prev) => [...prev, ev.payload]);
        },
      );
      const u2 = await listen<ConfigSearchSummaryDto>(
        `config-search-done::${id}`,
        (ev) => {
          setSearchSummary(ev.payload);
        },
      );
      searchUnlistenersRef.current = [u1, u2];
    } catch (e) {
      pushToast("error", `Search failed: ${e}`);
      detachSearchListeners();
      return;
    }
    setActiveSearchId(id);
    setSearchActive(true);
    try {
      await api.configSearchStart(id, {
        text: trimmed,
        regex: searchRegex,
        case_sensitive: false,
      });
    } catch (e) {
      pushToast("error", `Search failed: ${e}`);
      detachSearchListeners();
      setSearchActive(false);
      setActiveSearchId(null);
    }
  }, [searchQuery, searchRegex, activeSearchId, pushToast, detachSearchListeners]);

  const cancelSearch = useCallback(async () => {
    if (activeSearchId) {
      try {
        await api.configSearchCancel(activeSearchId);
      } catch {
        // ignore
      }
    }
    detachSearchListeners();
    setActiveSearchId(null);
    setSearchActive(false);
  }, [activeSearchId, detachSearchListeners]);

  const clearSearch = useCallback(async () => {
    await cancelSearch();
    setSearchQuery("");
    setSearchHits([]);
    setSearchSummary(null);
  }, [cancelSearch]);

  // Section-unmount cleanup: drop any still-attached search listeners
  // so they can't fire on a dead component.
  useEffect(() => {
    return () => {
      detachSearchListeners();
    };
  }, [detachSearchListeners]);

  const selectedFile = useMemo<ConfigFileNodeDto | null>(() => {
    if (!tree || !selectedId) return null;
    for (const scope of tree.scopes) {
      const hit = scope.files.find((f) => f.id === selectedId);
      if (hit) return hit;
    }
    return null;
  }, [tree, selectedId]);

  const openFileInEditor = useCallback(
    async (editorId: string | null) => {
      if (!selectedFile) return;
      try {
        await api.configOpenInEditorPath(
          selectedFile.abs_path,
          editorId,
          selectedFile.kind as ConfigKind,
        );
      } catch (err) {
        pushToast("error", String(err));
      }
    },
    [selectedFile, pushToast],
  );

  const openConfigHome = useCallback(
    async (editorId: string | null) => {
      if (!tree?.config_home_dir) return;
      try {
        await api.configOpenInEditorPath(tree.config_home_dir, editorId, null);
      } catch (err) {
        pushToast("error", String(err));
      }
    },
    [tree?.config_home_dir, pushToast],
  );

  const pickOther = useCallback(async () => {
    try {
      const picked = await openDialog({
        multiple: false,
        title: "Choose editor binary",
      });
      if (typeof picked !== "string") return;
      pushToast("info", `Custom editor support lands with P8: ${picked}`);
    } catch {
      pushToast("error", "Could not open file picker");
    }
  }, [pushToast]);

  const setDefault = useCallback(
    async (kind: ConfigKind | null, editorId: string) => {
      try {
        await api.configSetEditorDefault(kind, editorId);
        const next = await api.configGetEditorDefaults();
        setDefaults(next);
        pushToast(
          "info",
          kind
            ? `Default editor for ${kind} set to ${editorId}`
            : `Fallback editor set to ${editorId}`,
        );
      } catch (err) {
        pushToast("error", String(err));
      }
    },
    [pushToast],
  );

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        minHeight: 0,
      }}
    >
      {forcedAnchor ? (
        // Embedded mode: the outer shell (project tabs or Global
        // section) already owns the title. Render a slim status strip
        // instead of a full ScreenHeader so the tree/preview grid
        // dominates the vertical space.
        <EmbeddedStatusStrip
          artifactCount={
            tree
              ? tree.scopes.reduce((n, s) => n + s.recursive_count, 0)
              : null
          }
          updating={watcherDirty}
          onRefresh={() => void refreshTree()}
        />
      ) : (
      <ScreenHeader
        title="Config"
        subtitle={
          tree
            ? (() => {
                const total = tree.scopes.reduce(
                  (n, s) => n + s.recursive_count,
                  0,
                );
                const locus =
                  anchor.kind === "folder" ? anchor.path : "Global only";
                return `${total} artifact${total === 1 ? "" : "s"} · ${locus}${watcherDirty ? " · updating…" : ""}`;
              })()
            : "Read-only browser over Claude Code's filesystem artifacts."
        }
        actions={
          <div style={{ display: "flex", gap: "var(--sp-6)", alignItems: "center" }}>
            <AnchorPicker
              anchor={anchor}
              recent={recentProjects}
              onGlobal={() => chooseAnchor({ kind: "global" })}
              onFolder={(path) => chooseAnchor({ kind: "folder", path })}
              onPickFolder={() => void pickFolderAnchor()}
            />
            <Button
              variant="ghost"
              glyph={NF.refresh}
              onClick={() => void refreshTree()}
              title="Re-scan on-disk artifacts"
            >
              Refresh
            </Button>
          </div>
        }
      />
      )}

      <div
        style={{
          flex: 1,
          display: "grid",
          // When ConfigSection is nested inside ProjectsSection's
          // CONFIG tab there's already a project-filter rail to the
          // left; pull the artifact tree in tighter so the detail
          // pane has room for tables (Effective MCP, Hooks).
          gridTemplateColumns: forcedAnchor
            ? "var(--config-tree-width-nested) minmax(0, 1fr)"
            : "var(--config-tree-width) minmax(0, 1fr)",
          minHeight: 0,
        }}
      >
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            minHeight: 0,
            minWidth: 0,
            background: "var(--bg-sunken)",
          }}
        >
          <SearchBar
            value={searchQuery}
            onChange={setSearchQuery}
            regex={searchRegex}
            onToggleRegex={() => setSearchRegex((v) => !v)}
            onSubmit={() => void startSearch()}
            onClear={() => void clearSearch()}
            inputRef={searchInputRef}
            active={searchActive}
          />

          {searchActive ? (
            <SearchResultsPane
              hits={searchHits}
              summary={searchSummary}
              tree={tree}
              selectedId={selectedId}
              onSelect={(id) => onSubRouteChange(id ? `node:${id}` : null)}
            />
          ) : (
            <ConfigTreePane
              tree={tree}
              loadError={loadError}
              selectedId={selectedId}
              onSelect={(id) => onSubRouteChange(id ? `node:${id}` : null)}
              globalOnly={anchor.kind === "global"}
              hooksCount={hooksCount}
            />
          )}
        </div>

        <div
          style={{
            display: "flex",
            flexDirection: "column",
            minHeight: 0,
            minWidth: 0,
            borderLeft: "var(--bw-hair) solid var(--line)",
          }}
        >
          {virtualRoute === "effective-settings" ? (
            <EffectiveShell
              title="Effective settings"
              subtitle="Merged view of every enabled source. Hover a value to see contributors."
              scopeLabel={effectiveScopeLabel(anchor)}
              onClose={() => onSubRouteChange(null)}
            >
              <EffectiveRenderer cwd={tree?.cwd ?? null} />
            </EffectiveShell>
          ) : virtualRoute === "effective-mcp" ? (
            <EffectiveShell
              title="Effective MCP"
              subtitle="MCP servers CC would see, per simulation mode."
              scopeLabel={effectiveScopeLabel(anchor)}
              onClose={() => onSubRouteChange(null)}
            >
              <EffectiveMcpRenderer cwd={tree?.cwd ?? null} />
            </EffectiveShell>
          ) : virtualRoute === "hooks" ? (
            <EffectiveShell
              title="Hooks"
              subtitle="Registered hooks across every enabled settings layer. One row per matcher → command."
              scopeLabel={effectiveScopeLabel(anchor)}
              onClose={() => onSubRouteChange(null)}
            >
              <HooksRenderer cwd={tree?.cwd ?? null} />
            </EffectiveShell>
          ) : selectedFile ? (
            <FilePreview
              file={selectedFile}
              preview={preview}
              previewError={previewError}
              editors={editors}
              defaults={defaults}
              onOpen={openFileInEditor}
              onPickOther={pickOther}
              onSetDefault={setDefault}
              onRefreshEditors={refreshEditors}
              onClose={() => onSubRouteChange(null)}
            />
          ) : (
            <ConfigHomePane
              configHomeDir={tree?.config_home_dir ?? null}
              editors={editors}
              defaults={defaults}
              onOpen={openConfigHome}
              onPickOther={pickOther}
              onSetDefault={setDefault}
              onRefreshEditors={refreshEditors}
            />
          )}
        </div>
      </div>
    </div>
  );
}

// ---------- Search bar -----------------------------------------------

function SearchBar({
  value,
  onChange,
  regex,
  onToggleRegex,
  onSubmit,
  onClear,
  inputRef,
  active,
}: {
  value: string;
  onChange: (v: string) => void;
  regex: boolean;
  onToggleRegex: () => void;
  onSubmit: () => void;
  onClear: () => void;
  inputRef: React.RefObject<HTMLInputElement | null>;
  active: boolean;
}) {
  return (
    <div
      style={{
        padding: "var(--sp-8) var(--sp-12)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        display: "flex",
        gap: "var(--sp-6)",
        alignItems: "center",
        flexShrink: 0,
      }}
    >
      <div style={{ flex: 1, minWidth: 0 }}>
        <Input
          glyph={NF.search}
          placeholder="Search contents…"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") onSubmit();
            if (e.key === "Escape") onClear();
          }}
          aria-label="Search contents"
          inputRef={inputRef}
          suffix={
            value || active ? (
              <IconButton
                glyph={NF.x}
                onClick={onClear}
                size="sm"
                title="Clear search"
                aria-label="Clear search"
              />
            ) : undefined
          }
        />
      </div>
      <FilterChip
        active={regex}
        onToggle={onToggleRegex}
        title="Toggle regex mode"
        aria-label="Regex mode"
      >
        .*
      </FilterChip>
    </div>
  );
}

// ---------- Tree pane (virtualized) ----------------------------------
//
// Flat CC-native layout. No meta-section headers ("Effective" /
// "Definitions" / "Sources") — those were engineering vocabulary.
// Groups are separated by thin dividers; each row is a first-class
// Claude Code concept.
//
// Top block — project-scoped merged views (hidden in global-only):
//   Settings · MCP servers · Hooks · Memory
//
// Middle block — kind-bucketed authored artifacts (cross-scope),
// badged with origin (U / P / L / Pl …):
//   Agents · Skills · Commands · Output styles · Workflows · Rules
//   · Keybindings · Statusline
//
// Plugins — promoted out of the raw-files bucket. Each plugin is a
// parent bundle; expanding one shows its contributed files (agents,
// skills, commands, hooks, manifest) flattened by kind.
//
// Files — the old "Sources" scope-by-scope tree, kept as a
// debugging escape hatch. Collapsed by default.
//
// Hooks count comes from parsing effective settings' `hooks` field.
// Plugin bundle grouping uses each file's abs_path prefix against
// the plugin root (computed client-side from existing scope data).

type TreeRow =
  | {
      kind: "group";
      id: string;
      label: string;
      count?: number;
      expanded: boolean;
      depth: 0 | 1;
    }
  | { kind: "virtual-row"; id: string; label: string; count?: number; depth: 0 | 1 | 2 }
  | {
      kind: "file";
      file: ConfigFileNodeDto;
      scopeBadge: ScopeBadge | null;
      depth: 1 | 2;
    }
  | { kind: "divider"; id: string };

const ROW_HEIGHT = 26;

function ConfigTreePane({
  tree,
  loadError,
  selectedId,
  onSelect,
  globalOnly,
  hooksCount,
}: {
  tree: ConfigTreeDto | null;
  loadError: string | null;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  globalOnly: boolean;
  /**
   * Number of hooks found in merged effective settings, or `null` if
   * the Effective settings haven't resolved yet / are unavailable
   * (global-only mode). Controls whether the Hooks row is rendered.
   */
  hooksCount: number | null;
}) {
  const [expanded, setExpanded] = useState<Record<string, boolean>>({
    [FILES_GROUP_ID]: false,
    [PLUGINS_GROUP_ID]: false,
  });
  const parentRef = useRef<HTMLDivElement | null>(null);

  // Default expansion rules:
  //  - Every definition kind-group is closed so the initial tree is
  //    a dense summary view, not a wall of leaf rows.
  //  - Every scope under Files is closed — the whole Files block is
  //    collapsed by default anyway, but if the user opens it they'd
  //    rather see scope labels than instantly re-explode into files.
  //  - Each plugin's sub-group is closed.
  useEffect(() => {
    if (!tree) return;
    setExpanded((prev) => {
      const next = { ...prev };
      tree.scopes.forEach((s) => {
        if (!(s.id in next)) next[s.id] = false;
      });
      for (const k of DEFINITION_KINDS) {
        const gid = `def:${k}`;
        if (!(gid in next)) next[gid] = false;
      }
      return next;
    });
  }, [tree]);

  const rows: TreeRow[] = useMemo(() => {
    const out: TreeRow[] = [];

    // --- Merged views (project-scoped) --------------------------------
    // Hidden in global-only mode — the backend refuses effective_*
    // without a cwd and there's no project to walk CLAUDE.md from.
    if (!globalOnly) {
      out.push({
        kind: "virtual-row",
        id: EFFECTIVE_SETTINGS_ROUTE,
        label: "Settings",
        depth: 0,
      });
      out.push({
        kind: "virtual-row",
        id: EFFECTIVE_MCP_ROUTE,
        label: "MCP servers",
        depth: 0,
      });
      if (hooksCount != null && hooksCount > 0) {
        out.push({
          kind: "virtual-row",
          id: HOOKS_ROUTE,
          label: "Hooks",
          count: hooksCount,
          depth: 0,
        });
      }
    }

    if (!tree) return out;

    // --- Definitions (cross-scope, by kind) ---------------------------
    // Plugin-contributed files are EXCLUDED here — they surface under
    // the dedicated Plugins block below so the top-level counts mean
    // "things you authored" rather than "everything loaded."
    const byKind = new Map<string, { file: ConfigFileNodeDto; badge: ScopeBadge }[]>();
    for (const scope of tree.scopes) {
      if (scope.scope_type === "plugin" || scope.scope_type === "plugin_base") {
        continue;
      }
      const badge = scopeBadgeFor(scope.scope_type);
      for (const f of scope.files) {
        if ((DEFINITION_KINDS as readonly string[]).includes(f.kind)) {
          const bucket = byKind.get(f.kind) ?? [];
          bucket.push({ file: f, badge });
          byKind.set(f.kind, bucket);
        }
      }
    }
    for (const bucket of byKind.values()) {
      bucket.sort((a, b) => {
        const la = (a.file.summary_title ?? fileName(a.file.display_path)).toLowerCase();
        const lb = (b.file.summary_title ?? fileName(b.file.display_path)).toLowerCase();
        return la < lb ? -1 : la > lb ? 1 : 0;
      });
    }

    const hadMergedBlock = out.length > 0;
    const hasDefs = Array.from(byKind.values()).some((b) => b.length > 0);
    if (hasDefs) {
      if (hadMergedBlock) {
        out.push({ kind: "divider", id: "div:defs" });
      }
      for (const k of DEFINITION_KINDS) {
        const bucket = byKind.get(k);
        if (!bucket || bucket.length === 0) continue;
        const gid = `def:${k}`;
        const gOpen = expanded[gid] ?? false;
        out.push({
          kind: "group",
          id: gid,
          label: DEFINITION_KIND_LABEL[k] ?? kindLabel(k),
          count: bucket.length,
          expanded: gOpen,
          depth: 0,
        });
        if (gOpen) {
          for (const { file, badge } of bucket) {
            out.push({ kind: "file", file, scopeBadge: badge, depth: 1 });
          }
        }
      }
    }

    // --- Plugins (first-class bundle) ---------------------------------
    const pluginFiles = tree.scopes
      .filter((s) => s.scope_type === "plugin" || s.scope_type === "plugin_base")
      .flatMap((s) => s.files);
    if (pluginFiles.length > 0) {
      const plugins = groupFilesByPlugin(pluginFiles);
      const plugOpen = expanded[PLUGINS_GROUP_ID] ?? false;
      out.push({ kind: "divider", id: "div:plugins" });
      out.push({
        kind: "group",
        id: PLUGINS_GROUP_ID,
        label: "Plugins",
        count: plugins.length,
        expanded: plugOpen,
        depth: 0,
      });
      if (plugOpen) {
        for (const p of plugins) {
          const pid = `plug:${p.id}`;
          const pOpen = expanded[pid] ?? false;
          out.push({
            kind: "group",
            id: pid,
            label: p.label,
            count: p.files.length,
            expanded: pOpen,
            depth: 1,
          });
          if (pOpen) {
            for (const f of p.files) {
              out.push({ kind: "file", file: f, scopeBadge: null, depth: 2 });
            }
          }
        }
      }
    }

    // --- Files (scope-by-scope debug view) ----------------------------
    // Everything user-visible above is derived from these same scopes;
    // Files is the escape hatch for "show me the raw file layout."
    if (tree.scopes.length > 0) {
      out.push({ kind: "divider", id: "div:files" });
      const filesOpen = expanded[FILES_GROUP_ID] ?? false;
      out.push({
        kind: "group",
        id: FILES_GROUP_ID,
        label: "Files",
        count: tree.scopes.length,
        expanded: filesOpen,
        depth: 0,
      });
      if (filesOpen) {
        for (const scope of tree.scopes) {
          const sOpen = expanded[scope.id] ?? false;
          out.push({
            kind: "group",
            id: scope.id,
            label: cleanScopeLabel(scope.label),
            count: scope.recursive_count,
            expanded: sOpen,
            depth: 1,
          });
          if (sOpen) {
            for (const f of scope.files) {
              out.push({ kind: "file", file: f, scopeBadge: null, depth: 2 });
            }
          }
        }
      }
    }

    return out;
  }, [tree, expanded, globalOnly, hooksCount]);

  const virt = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 12,
  });

  if (loadError) {
    return (
      <div
        style={{
          padding: "var(--sp-20)",
          fontSize: "var(--fs-sm)",
          color: "var(--danger)",
        }}
      >
        Scan failed: {loadError}
      </div>
    );
  }
  if (!tree) {
    return (
      <div style={{ padding: "var(--sp-20)", color: "var(--fg-faint)" }}>
        Scanning…
      </div>
    );
  }
  if (tree.scopes.length === 0) {
    return (
      <div
        style={{
          padding: "var(--sp-20)",
          fontSize: "var(--fs-sm)",
          color: "var(--fg-faint)",
        }}
      >
        No Claude config found at this cwd or ~/.claude.
      </div>
    );
  }

  const toggle = (id: string) =>
    setExpanded((p) => ({ ...p, [id]: !(p[id] ?? false) }));

  return (
    <div
      ref={parentRef}
      role="tree"
      aria-label="Config artifacts"
      style={{
        flex: 1,
        minHeight: 0,
        overflowY: "auto",
        padding: "var(--sp-8) 0",
      }}
    >
      <div
        style={{
          position: "relative",
          height: `${virt.getTotalSize()}px`,
          width: "100%",
        }}
      >
        {virt.getVirtualItems().map((vi) => {
          const row = rows[vi.index];
          return (
            <div
              key={vi.key}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                right: 0,
                transform: `translateY(${vi.start}px)`,
                height: `${ROW_HEIGHT}px`,
              }}
            >
              <TreeRowView
                row={row}
                selectedId={selectedId}
                onSelect={onSelect}
                onToggle={toggle}
              />
            </div>
          );
        })}
      </div>
    </div>
  );
}

function TreeRowView({
  row,
  selectedId,
  onSelect,
  onToggle,
}: {
  row: TreeRow;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  onToggle: (id: string) => void;
}) {
  if (row.kind === "divider") {
    return <TreeDivider />;
  }
  if (row.kind === "group") {
    return (
      <GroupHeaderButton
        expanded={row.expanded}
        label={row.label}
        count={row.count}
        onToggle={() => onToggle(row.id)}
        depth={row.depth}
      />
    );
  }
  if (row.kind === "virtual-row") {
    return (
      <VirtualRowButton
        selected={selectedId === row.id}
        label={row.label}
        count={row.count}
        onSelect={() => onSelect(row.id)}
        depth={row.depth}
      />
    );
  }
  const label = row.file.summary_title ?? fileName(row.file.display_path);
  return (
    <FileRowButton
      selected={selectedId === row.file.id}
      label={label}
      onSelect={() => onSelect(row.file.id)}
      title={row.file.abs_path}
      issuesCount={row.file.issues.length}
      issuesTitle={row.file.issues.join("; ")}
      includeDepth={row.file.include_depth}
      scopeBadge={row.scopeBadge ?? undefined}
      depth={row.depth}
    />
  );
}

function TreeDivider() {
  return (
    <div
      role="separator"
      aria-orientation="horizontal"
      style={{
        display: "flex",
        alignItems: "center",
        height: "100%",
        padding: "0 var(--sp-12)",
      }}
    >
      <div
        style={{
          flex: 1,
          height: "var(--bw-hair)",
          background: "var(--line)",
        }}
      />
    </div>
  );
}

// ---------- Group / virtual / file rows -------------------------------

function GroupHeaderButton({
  expanded,
  label,
  count,
  onToggle,
  depth,
}: {
  expanded: boolean;
  label: string;
  count?: number;
  onToggle: () => void;
  depth: 0 | 1;
}) {
  // Depth 0 = top-level group (Agents / Skills / Plugins / Files).
  // Depth 1 = nested group inside Plugins or Files (per-plugin bundle
  // or per-scope file list).
  const leftPad = depth === 0 ? "var(--sp-12)" : "var(--sp-24)";
  return (
    <button
      type="button"
      role="treeitem"
      aria-expanded={expanded}
      onClick={onToggle}
      className="pm-focus"
      style={{
        display: "flex",
        alignItems: "center",
        width: "100%",
        height: "100%",
        gap: "var(--sp-6)",
        padding: `0 var(--sp-12) 0 ${leftPad}`,
        background: "transparent",
        border: "none",
        cursor: "pointer",
        color: "var(--fg)",
        fontSize: "var(--fs-xs)",
        fontWeight: depth === 0 ? 500 : 400,
        textAlign: "left",
      }}
    >
      <Glyph g={expanded ? NF.chevronD : NF.chevronR} color="var(--fg-muted)" />
      <span
        style={{
          flex: 1,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
      >
        {label}
      </span>
      {count != null && (
        <span
          style={{
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-faint)",
            fontWeight: 400,
          }}
        >
          {count}
        </span>
      )}
    </button>
  );
}

function VirtualRowButton({
  selected,
  label,
  count,
  onSelect,
  depth,
}: {
  selected: boolean;
  label: string;
  count?: number;
  onSelect: () => void;
  depth: 0 | 1 | 2;
}) {
  // depth 0 — top-level merged views (Settings, MCP, Hooks, Memory).
  // Given no chevron, pad the left edge to the same spot label would
  // start under an expanded group chevron, keeping vertical alignment
  // with the group rows below.
  const leftPad =
    depth === 0 ? "var(--sp-24)" : depth === 1 ? "var(--sp-32)" : "var(--sp-40)";
  return (
    <button
      type="button"
      role="treeitem"
      aria-selected={selected}
      onClick={onSelect}
      className="pm-focus"
      style={{
        display: "flex",
        alignItems: "center",
        width: "100%",
        height: "100%",
        gap: "var(--sp-6)",
        padding: `0 var(--sp-12) 0 ${leftPad}`,
        background: selected ? "var(--bg-active)" : "transparent",
        color: selected ? "var(--accent-ink)" : "var(--fg)",
        border: "none",
        cursor: "pointer",
        fontSize: "var(--fs-xs)",
        fontWeight: depth === 0 ? 500 : 400,
        textAlign: "left",
      }}
    >
      <span
        style={{
          flex: 1,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
      >
        {label}
      </span>
      {count != null && (
        <span
          style={{
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-faint)",
            fontWeight: 400,
          }}
        >
          {count}
        </span>
      )}
    </button>
  );
}

function FileRowButton({
  selected,
  label,
  onSelect,
  title,
  issuesCount,
  issuesTitle,
  includeDepth,
  scopeBadge,
  depth,
}: {
  selected: boolean;
  label: string;
  onSelect: () => void;
  title?: string;
  issuesCount?: number;
  issuesTitle?: string;
  includeDepth?: number;
  scopeBadge?: ScopeBadge;
  depth: number;
}) {
  // Base indent derives from tree depth (section/group/file). `@include`
  // chains nest further under their parent file.
  const basePad =
    depth >= 2 ? "var(--sp-32)" : depth === 1 ? "var(--sp-20)" : "var(--sp-12)";
  const inc = includeDepth ?? 0;
  const leftPad = inc === 0 ? basePad : `calc(${basePad} + ${inc * 12}px)`;
  return (
    <button
      type="button"
      role="treeitem"
      aria-selected={selected}
      onClick={onSelect}
      className="pm-focus"
      title={title}
      style={{
        display: "flex",
        alignItems: "center",
        width: "100%",
        height: "100%",
        gap: "var(--sp-6)",
        padding: `0 var(--sp-12) 0 ${leftPad}`,
        background: selected ? "var(--bg-active)" : "transparent",
        color: selected ? "var(--accent-ink)" : "var(--fg)",
        border: "none",
        cursor: "pointer",
        fontSize: "var(--fs-xs)",
        textAlign: "left",
      }}
    >
      {inc > 0 && (
        <span
          aria-hidden
          style={{
            fontFamily: "var(--mono)",
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-faint)",
          }}
        >
          @
        </span>
      )}
      <span
        style={{
          flex: 1,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
      >
        {label}
      </span>
      {scopeBadge && <ScopeBadgeChip badge={scopeBadge} />}
      {issuesCount != null && issuesCount > 0 && (
        <span title={issuesTitle} aria-label={`${issuesCount} issue${issuesCount === 1 ? "" : "s"}`}>
          <Glyph g={NF.warn} color="var(--danger)" />
        </span>
      )}
    </button>
  );
}

// ---------- Scope badges ----------------------------------------------
//
// One-char chips tell Definition rows which scope they came from,
// without forcing the user to swap to the Sources tree. `scope_type`
// is the serde-tagged discriminator on the Rust enum (snake_case).

interface ScopeBadge {
  short: string;
  full: string;
}

function scopeBadgeFor(scopeType: string): ScopeBadge {
  switch (scopeType) {
    case "user":
      return { short: "U", full: "User (~/.claude)" };
    case "project":
      return { short: "P", full: "Project (cwd/.claude)" };
    case "local":
      return { short: "L", full: "Local overrides" };
    case "flag":
      return { short: "F", full: "CLI flag override" };
    case "plugin":
      return { short: "Pl", full: "Plugin" };
    case "plugin_base":
      return { short: "Pb", full: "Plugin base" };
    case "policy":
      return { short: "Po", full: "Managed policy" };
    case "claude_md_dir":
      return { short: "C", full: "CLAUDE.md walk" };
    case "memory_current":
      return { short: "M", full: "Memory (this project)" };
    case "memory_other":
      return { short: "m", full: "Memory (other project)" };
    case "redacted_user_config":
      return { short: "G", full: "Global config" };
    case "effective":
      return { short: "E", full: "Effective" };
    default:
      return { short: "·", full: scopeType };
  }
}

function ScopeBadgeChip({ badge }: { badge: ScopeBadge }) {
  return (
    <span
      title={badge.full}
      aria-label={badge.full}
      style={{
        fontFamily: "var(--mono)",
        fontSize: "var(--fs-2xs)",
        fontWeight: 600,
        lineHeight: "var(--lh-flat)",
        padding: "var(--sp-2) var(--sp-5)",
        borderRadius: "var(--r-sm)",
        border: "var(--bw-hair) solid var(--line)",
        color: "var(--fg-muted)",
        background: "var(--bg-sunken)",
        minWidth: "1.4em",
        textAlign: "center",
      }}
    >
      {badge.short}
    </span>
  );
}

// ---------- Plugin bundle grouping ------------------------------------
//
// Plugin files come from the backend in a single flat scope. To render
// them as first-class bundles we have to partition by plugin root —
// every plugin under `~/.claude/plugins/<id>/…` or
// `<any-scope>/plugins/<id>/…` groups its agents/skills/commands
// together. We use the longest matching root prefix among the files
// themselves: the deepest ancestor directory whose basename is the
// plugin id. This is heuristic but stable for how CC ships plugins
// today. If the backend ever exposes a `plugin_id` field on FileNode
// this can be replaced with a direct bucketing.
interface PluginBundle {
  id: string;
  label: string;
  files: ConfigFileNodeDto[];
}

function groupFilesByPlugin(files: ConfigFileNodeDto[]): PluginBundle[] {
  const byId = new Map<string, PluginBundle>();
  for (const f of files) {
    const id = pluginIdFromPath(f.abs_path);
    const bucket = byId.get(id) ?? { id, label: id, files: [] };
    bucket.files.push(f);
    byId.set(id, bucket);
  }
  const out = Array.from(byId.values());
  // Sort within each plugin: manifest-like files first, then the rest
  // alphabetically. Manifests are the user's first landmark per plugin.
  for (const b of out) {
    b.files.sort((a, b2) => {
      const ma = isManifestLike(a) ? 0 : 1;
      const mb = isManifestLike(b2) ? 0 : 1;
      if (ma !== mb) return ma - mb;
      const la = (a.summary_title ?? fileName(a.display_path)).toLowerCase();
      const lb = (b2.summary_title ?? fileName(b2.display_path)).toLowerCase();
      return la < lb ? -1 : la > lb ? 1 : 0;
    });
  }
  out.sort((a, b) => a.label.localeCompare(b.label));
  return out;
}

// CC stores plugins at:
//   ~/.claude/plugins/cache/<marketplace>/<plugin>/<version>/…
// The user-meaningful id is `<plugin>` (3 segments after `plugins`).
// Fall back to `<marketplace>/<plugin>` shapes for older layouts that
// omit the cache or version level, then to a stable sentinel.
function pluginIdFromPath(absPath: string): string {
  const segs = absPath.split(/[/\\]/).filter(Boolean);
  const idx = segs.lastIndexOf("plugins");
  if (idx === -1) return "(unknown plugin)";
  // cache/<marketplace>/<plugin>/<version>/…  → segs[idx + 3]
  if (segs[idx + 1] === "cache" && segs.length > idx + 3) {
    return segs[idx + 3];
  }
  // Older: plugins/<marketplace>/<plugin>/…
  if (segs.length > idx + 2) return segs[idx + 2];
  // Even older: plugins/<plugin>/…
  if (segs.length > idx + 1) return segs[idx + 1];
  return "(unknown plugin)";
}

function isManifestLike(f: ConfigFileNodeDto): boolean {
  const base = fileName(f.display_path).toLowerCase();
  return base === "plugin.json" || base === "manifest.json";
}

// ---------- Scope label cleanup ---------------------------------------
//
// Backend labels carry their provenance ("User (~/.claude)"). In the
// redesigned tree the section structure already conveys that, so the
// scope row should read as a short name; the full path moves into the
// row's `title` tooltip (set by the backend-provided `abs_path` at the
// file-row level). This purely cosmetic map keeps labels consistent.
function cleanScopeLabel(label: string): string {
  // Exact rewrites first — cheaper and more predictable than regexes.
  const exact: Record<string, string> = {
    "User (~/.claude)": "User",
    "Project (cwd/.claude)": "Project",
    "Local (settings.local.json + CLAUDE.local.md)": "Local",
    "MCP (.mcp.json walk)": "MCP walk",
    "Policy (managed-settings)": "Managed policy",
    "Memory (this project)": "Memory",
    "Memory (other projects)": "Other projects memory",
    "Global config (redacted)": "Global config",
    Plugins: "Plugins",
  };
  if (label in exact) return exact[label];
  // `CLAUDE.md — /some/dir` and `CLAUDE.md — /some/dir (cwd)` stay as-is:
  // the per-directory path is the only way to tell duplicates apart.
  return label;
}

function fileName(path: string): string {
  const m = path.match(/([^/\\]+)$/);
  return m ? m[1] : path;
}

/**
 * Best-guess language hint for kinds that fall to the CodeRenderer.
 * The renderer also runs extension and shebang detection — this just
 * gives a sane default for kinds whose path doesn't carry a useful
 * extension (e.g., a `statusline` named `statusline` with no suffix).
 * Returning null lets the renderer's auto-detection take over.
 */
function codeHintForKind(kind: ConfigKind): string | null {
  switch (kind) {
    case "statusline":
      return "bash";
    case "hook":
      return null; // hook entries are JSON snippets in settings; let auto-detect run
    default:
      return null;
  }
}

// ---------- File preview ---------------------------------------------

function FilePreview({
  file,
  preview,
  previewError,
  editors,
  defaults,
  onOpen,
  onPickOther,
  onSetDefault,
  onRefreshEditors,
  onClose,
}: {
  file: ConfigFileNodeDto;
  preview: ConfigPreviewDto | null;
  previewError: string | null;
  editors: EditorCandidateDto[] | null;
  defaults: EditorDefaultsDto | null;
  onOpen: (editorId: string | null) => void;
  onPickOther: () => void;
  onSetDefault: (kind: ConfigKind | null, editorId: string) => void;
  onRefreshEditors: () => void;
  onClose?: () => void;
}) {
  const kind = file.kind as ConfigKind;
  const isMarkdown = MARKDOWN_KINDS.includes(kind);
  const isJson = JSON_KINDS.includes(kind);

  return (
    <div
      style={{
        flex: 1,
        display: "flex",
        flexDirection: "column",
        minHeight: 0,
      }}
    >
      <PreviewHeader
        title={file.summary_title ?? fileName(file.display_path)}
        subtitle={file.summary_description ?? kindLabel(file.kind)}
        path={file.abs_path}
        kind={kind}
        editors={editors}
        defaults={defaults}
        onOpen={onOpen}
        onPickOther={onPickOther}
        onSetDefault={onSetDefault}
        onRefreshEditors={onRefreshEditors}
        onClose={onClose}
      />
      {file.include_depth > 0 && file.included_by && (
        <IncludedByBanner
          includedBy={file.included_by}
          depth={file.include_depth}
        />
      )}
      <div
        style={{
          flex: 1,
          overflow: "auto",
          minHeight: 0,
          color: "var(--fg)",
          background: "var(--bg)",
        }}
      >
        {previewError ? (
          <div
            style={{
              padding: "var(--sp-16) var(--sp-20)",
              color: "var(--danger)",
              fontSize: "var(--fs-sm)",
            }}
          >
            Preview failed: {previewError}
          </div>
        ) : !preview ? (
          <div
            style={{
              padding: "var(--sp-16) var(--sp-20)",
              color: "var(--fg-faint)",
              fontSize: "var(--fs-sm)",
            }}
          >
            Loading…
          </div>
        ) : isMarkdown ? (
          <>
            <MarkdownRenderer body={preview.body_utf8} />
            {preview.truncated && <TruncationFooter onOpen={onOpen} />}
          </>
        ) : isJson ? (
          <>
            <JsonTreeRenderer body={preview.body_utf8} />
            {preview.truncated && <TruncationFooter onOpen={onOpen} />}
          </>
        ) : (
          <>
            <CodeRenderer
              body={preview.body_utf8}
              path={file.abs_path}
              defaultLang={codeHintForKind(kind)}
            />
            {preview.truncated && <TruncationFooter onOpen={onOpen} />}
          </>
        )}
      </div>
    </div>
  );
}

function IncludedByBanner({
  includedBy,
  depth,
}: {
  includedBy: string;
  depth: number;
}) {
  const parentName = fileName(includedBy);
  return (
    <div
      role="note"
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-8)",
        padding: "var(--sp-8) var(--sp-20)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        background: "var(--bg-sunken)",
        color: "var(--fg-muted)",
        fontSize: "var(--fs-xs)",
      }}
    >
      <Glyph g={NF.link} color="var(--fg-muted)" />
      <span>
        Loaded via <code style={{ fontFamily: "var(--mono)" }}>@include</code>{" "}
        (depth {depth}) from{" "}
        <code
          style={{ fontFamily: "var(--mono)" }}
          title={includedBy}
        >
          {parentName}
        </code>
      </span>
    </div>
  );
}

function TruncationFooter({
  onOpen,
}: {
  onOpen: (editorId: string | null) => void;
}) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        gap: "var(--sp-12)",
        padding: "var(--sp-10) var(--sp-20)",
        borderTop: "var(--bw-hair) solid var(--line)",
        background: "var(--bg-sunken)",
        color: "var(--fg-faint)",
        fontSize: "var(--fs-xs)",
        fontStyle: "italic",
      }}
    >
      <span>Preview truncated at 256 KB.</span>
      <Button variant="subtle" size="sm" onClick={() => onOpen(null)}>
        Open full file
      </Button>
    </div>
  );
}

// ---------- Home pane (no file selected) ------------------------------

function ConfigHomePane({
  configHomeDir,
  editors,
  defaults,
  onOpen,
  onPickOther,
  onSetDefault,
  onRefreshEditors,
}: {
  configHomeDir: string | null;
  editors: EditorCandidateDto[] | null;
  defaults: EditorDefaultsDto | null;
  onOpen: (editorId: string | null) => void;
  onPickOther: () => void;
  onSetDefault: (kind: ConfigKind | null, editorId: string) => void;
  onRefreshEditors: () => void;
}) {
  return (
    <div
      style={{
        flex: 1,
        display: "flex",
        flexDirection: "column",
        minHeight: 0,
      }}
    >
      <PreviewHeader
        title="Config home"
        subtitle="Pick a file on the left to preview it, or open the whole .claude/ folder."
        path={configHomeDir}
        kind={null}
        editors={editors}
        defaults={defaults}
        onOpen={onOpen}
        onPickOther={onPickOther}
        onSetDefault={onSetDefault}
        onRefreshEditors={onRefreshEditors}
      />
      <div
        style={{
          flex: 1,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          color: "var(--fg-faint)",
          fontSize: "var(--fs-sm)",
          padding: "var(--sp-32)",
          textAlign: "center",
        }}
      >
        Select an artifact from the tree on the left to preview it.
      </div>
    </div>
  );
}

// ---------- Effective shell (wraps renderers) -------------------------

function EffectiveShell({
  title,
  subtitle,
  scopeLabel,
  onClose,
  children,
}: {
  title: string;
  subtitle: string;
  /** Project / "Global" context shown next to the title so the user can
   *  read what scope this view applies to without scanning the
   *  surrounding chrome. `null` hides the chip. */
  scopeLabel?: string | null;
  /** Closes this view and returns the right pane to `ConfigHomePane`.
   *  Renders a small chevron-left affordance above the title. Mirrors
   *  `PreviewHeader.onClose` so the master/detail pane has a uniform
   *  back signal whether the user opened a file or a virtual route. */
  onClose?: () => void;
  children: React.ReactNode;
}) {
  return (
    <div
      style={{
        flex: 1,
        display: "flex",
        flexDirection: "column",
        minHeight: 0,
      }}
    >
      <header
        style={{
          padding: "var(--sp-16) var(--sp-20) var(--sp-12)",
          borderBottom: "var(--bw-hair) solid var(--line)",
        }}
      >
        {onClose && (
          <BackAffordance
            label="Artifacts"
            onClick={onClose}
            title="Back to artifact list"
            style={{ marginBottom: "var(--sp-6)" }}
          />
        )}
        <div
          style={{
            display: "flex",
            alignItems: "baseline",
            gap: "var(--sp-10)",
            flexWrap: "wrap",
          }}
        >
          <h2
            style={{
              margin: 0,
              fontSize: "var(--fs-lg)",
              fontWeight: 600,
              color: "var(--fg)",
            }}
          >
            {title}
          </h2>
          {scopeLabel && (
            <span
              className="mono-cap"
              title={scopeLabel}
              style={{
                fontSize: "var(--fs-2xs)",
                color: "var(--fg-faint)",
                letterSpacing: "var(--ls-wide)",
                textTransform: "uppercase",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
                maxWidth: "60%",
              }}
            >
              {scopeLabel}
            </span>
          )}
        </div>
        <div
          style={{
            marginTop: "var(--sp-4)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg-faint)",
          }}
        >
          {subtitle}
        </div>
      </header>
      {children}
    </div>
  );
}

// ---------- Kind labels ----------------------------------------------

const KIND_LABELS: Record<string, string> = {
  claude_md: "CLAUDE.md",
  settings: "settings.json",
  settings_local: "settings.local.json",
  managed_settings: "managed-settings.json",
  redacted_user_config: "Global config (redacted)",
  mcp_json: ".mcp.json",
  managed_mcp_json: "managed-mcp.json",
  agent: "Agent",
  skill: "Skill",
  command: "Command",
  output_style: "Output style",
  workflow: "Workflow",
  rule: "Rule",
  hook: "Hook",
  memory: "Memory",
  memory_index: "MEMORY.md",
  plugin: "Plugin",
  keybindings: "Keybindings",
  statusline: "Status line",
  effective_settings: "Effective settings",
  effective_mcp: "Effective MCP",
  other: "Other",
};

function kindLabel(kind: string): string {
  return KIND_LABELS[kind] ?? kind;
}

// ---------- Search results pane --------------------------------------

function SearchResultsPane({
  hits,
  summary,
  tree,
  selectedId,
  onSelect,
}: {
  hits: ConfigSearchHitDto[];
  summary: ConfigSearchSummaryDto | null;
  tree: ConfigTreeDto | null;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
}) {
  const idToLabel = useMemo(() => {
    const m = new Map<string, string>();
    if (!tree) return m;
    for (const s of tree.scopes) {
      for (const f of s.files) {
        m.set(f.id, f.summary_title ?? f.display_path);
      }
    }
    return m;
  }, [tree]);

  return (
    <div
      role="listbox"
      aria-label="Search results"
      style={{
        flex: 1,
        minHeight: 0,
        overflowY: "auto",
      }}
    >
      <div
        style={{
          padding: "var(--sp-6) var(--sp-12)",
          borderBottom: "var(--bw-hair) solid var(--line)",
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-faint)",
        }}
      >
        {summary
          ? `${summary.total_hits}${summary.capped ? " (capped)" : ""} hit${summary.total_hits === 1 ? "" : "s"}${summary.skipped_large > 0 ? ` · ${summary.skipped_large} file${summary.skipped_large === 1 ? "" : "s"} skipped (>2MB)` : ""}${summary.cancelled ? " · cancelled" : ""}`
          : `${hits.length} hit${hits.length === 1 ? "" : "s"} so far…`}
      </div>
      {hits.length === 0 && summary && summary.total_hits === 0 && (
        <div
          style={{
            padding: "var(--sp-20)",
            color: "var(--fg-faint)",
            fontSize: "var(--fs-sm)",
          }}
        >
          No matches.
        </div>
      )}
      {hits.map((hit, i) => (
        <button
          key={`${hit.node_id}-${i}-${hit.line_number}`}
          type="button"
          role="option"
          aria-selected={selectedId === hit.node_id}
          onClick={() => onSelect(hit.node_id)}
          className="pm-focus"
          style={{
            display: "block",
            width: "100%",
            textAlign: "left",
            padding: "var(--sp-6) var(--sp-12)",
            background:
              selectedId === hit.node_id ? "var(--bg-active)" : "transparent",
            color: "var(--fg)",
            border: "none",
            borderBottom: "var(--bw-hair) solid var(--line)",
            cursor: "pointer",
            fontSize: "var(--fs-xs)",
          }}
        >
          <div
            style={{
              display: "flex",
              gap: "var(--sp-6)",
              alignItems: "baseline",
            }}
          >
            <span
              style={{
                flex: 1,
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {idToLabel.get(hit.node_id) ?? hit.node_id}
            </span>
            <span
              style={{
                fontSize: "var(--fs-2xs)",
                color: "var(--fg-faint)",
                fontFamily: "var(--mono)",
              }}
            >
              :{hit.line_number}
            </span>
          </div>
          <pre
            style={{
              margin: "var(--sp-3) 0 0 0",
              fontFamily: "var(--mono)",
              fontSize: "var(--fs-2xs)",
              color: "var(--fg-muted)",
              whiteSpace: "pre-wrap",
              overflow: "hidden",
              overflowWrap: "anywhere",
              maxHeight: "var(--config-snippet-max-h)",
            }}
          >
            {hit.snippet}
          </pre>
        </button>
      ))}
    </div>
  );
}

// ---------- Anchor picker --------------------------------------------
//
// The anchor determines which project (or none) the Config page walks.
// It's the semantic pivot of the whole view — when the anchor changes,
// Project / Local / MCP-walk / CLAUDE.md-walk / Effective scopes all
// rebuild. Rendered as a dropdown next to Refresh in the header.
//
// Menu contents:
//   Recent (from `api.projectList()`, reachable only, most-recent first)
//   Pick folder…   — native directory picker
//   Global only    — explicitly drop every project scope

function AnchorPicker({
  anchor,
  recent,
  onGlobal,
  onFolder,
  onPickFolder,
}: {
  anchor: ConfigAnchor;
  recent: ProjectInfo[] | null;
  onGlobal: () => void;
  onFolder: (path: string) => void;
  onPickFolder: () => void;
}) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onDocClick = (e: MouseEvent) => {
      if (!rootRef.current) return;
      if (!rootRef.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  // Reachable, unique-by-path, most-recent first. Cap to keep the menu
  // a single scroll.
  const reachable = useMemo<ProjectInfo[]>(() => {
    if (!recent) return [];
    const seen = new Set<string>();
    const out: ProjectInfo[] = [];
    for (const p of recent) {
      if (!p.is_reachable) continue;
      if (seen.has(p.original_path)) continue;
      seen.add(p.original_path);
      out.push(p);
      if (out.length >= 12) break;
    }
    return out;
  }, [recent]);

  const isGlobal = anchor.kind === "global";

  return (
    <div ref={rootRef} style={{ position: "relative" }}>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="pm-focus"
        aria-haspopup="menu"
        aria-expanded={open}
        title={isGlobal ? "No project anchored" : anchor.path}
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-6)",
          padding: "var(--sp-4) var(--sp-10)",
          borderRadius: "var(--r-2)",
          border: "var(--bw-hair) solid var(--line)",
          background: "var(--bg)",
          color: "var(--fg)",
          fontSize: "var(--fs-xs)",
          cursor: "pointer",
          maxWidth: "240px",
        }}
      >
        <Glyph g={isGlobal ? NF.globe : NF.folder} color="var(--fg-muted)" />
        <span
          style={{
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {anchorLabel(anchor)}
        </span>
        <Glyph g={NF.chevronD} color="var(--fg-faint)" />
      </button>
      {open && (
        <div
          role="menu"
          aria-label="Project anchor"
          style={{
            position: "absolute",
            top: "calc(100% + var(--sp-4))",
            right: 0,
            zIndex: "var(--z-popover)" as unknown as number,
            minWidth: "var(--config-menu-min-width)",
            maxHeight: "var(--config-menu-max-height)",
            overflowY: "auto",
            background: "var(--bg-raised)",
            border: "var(--bw-hair) solid var(--line-strong)",
            borderRadius: "var(--r-2)",
            boxShadow: "var(--shadow-popover)",
            padding: "var(--sp-4) 0",
          }}
        >
          {reachable.length > 0 && (
            <AnchorMenuGroup label="Recent projects">
              {reachable.map((p) => (
                <AnchorMenuItem
                  key={p.original_path}
                  selected={
                    anchor.kind === "folder" && anchor.path === p.original_path
                  }
                  glyph={NF.folder}
                  title={p.original_path}
                  subtitle={p.original_path}
                  onClick={() => {
                    onFolder(p.original_path);
                    setOpen(false);
                  }}
                >
                  {baseName(p.original_path)}
                </AnchorMenuItem>
              ))}
            </AnchorMenuGroup>
          )}
          <div
            role="separator"
            aria-orientation="horizontal"
            style={{
              height: "var(--bw-hair)",
              background: "var(--line)",
              margin: "var(--sp-4) 0",
            }}
          />
          <AnchorMenuItem
            glyph={NF.folderOpen}
            onClick={() => {
              setOpen(false);
              onPickFolder();
            }}
          >
            Pick folder…
          </AnchorMenuItem>
          <AnchorMenuItem
            selected={isGlobal}
            glyph={NF.globe}
            subtitle="Hide project / local / CLAUDE.md-walk scopes"
            onClick={() => {
              onGlobal();
              setOpen(false);
            }}
          >
            Global only
          </AnchorMenuItem>
        </div>
      )}
    </div>
  );
}

function AnchorMenuGroup({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <div
        className="mono-cap"
        style={{
          padding: "var(--sp-6) var(--sp-12) var(--sp-4)",
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-faint)",
          letterSpacing: "0.08em",
          textTransform: "uppercase",
        }}
      >
        {label}
      </div>
      {children}
    </div>
  );
}

function AnchorMenuItem({
  selected,
  glyph,
  subtitle,
  title,
  onClick,
  children,
}: {
  selected?: boolean;
  glyph: NfIcon;
  subtitle?: string;
  title?: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      onClick={onClick}
      className="pm-focus"
      title={title}
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-8)",
        width: "100%",
        padding: "var(--sp-6) var(--sp-12)",
        background: selected ? "var(--bg-active)" : "transparent",
        color: selected ? "var(--accent-ink)" : "var(--fg)",
        border: "none",
        cursor: "pointer",
        fontSize: "var(--fs-xs)",
        textAlign: "left",
      }}
    >
      <Glyph g={glyph} color="var(--fg-muted)" />
      <div style={{ display: "flex", flexDirection: "column", minWidth: 0, flex: 1 }}>
        <span
          style={{
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {children}
        </span>
        {subtitle && (
          <span
            style={{
              fontSize: "var(--fs-2xs)",
              color: "var(--fg-faint)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {subtitle}
          </span>
        )}
      </div>
      {selected && <Glyph g={NF.check} color="var(--accent-ink)" />}
    </button>
  );
}

function baseName(path: string): string {
  const m = path.match(/([^/\\]+)[/\\]?$/);
  return m ? m[1] : path;
}
