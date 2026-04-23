import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { useVirtualizer } from "@tanstack/react-virtual";
import { api } from "../api";
import type {
  ConfigFileNodeDto,
  ConfigKind,
  ConfigPreviewDto,
  ConfigScopeNodeDto,
  ConfigSearchHitDto,
  ConfigSearchSummaryDto,
  ConfigTreeDto,
  EditorCandidateDto,
  EditorDefaultsDto,
} from "../types";
import { ScreenHeader } from "../shell/ScreenHeader";
import { PreviewHeader } from "../components/primitives/PreviewHeader";
import { Button } from "../components/primitives/Button";
import { IconButton } from "../components/primitives/IconButton";
import { FilterChip } from "../components/primitives/FilterChip";
import { Input } from "../components/primitives/Input";
import { Glyph } from "../components/primitives/Glyph";
import { NF } from "../icons";
import { EffectiveRenderer } from "./config/EffectiveRenderer";
import { EffectiveMcpRenderer } from "./config/EffectiveMcpRenderer";
import { MarkdownRenderer } from "./config/MarkdownRenderer";
import { JsonTreeRenderer } from "./config/JsonTreeRenderer";
import { useConfigTree } from "../hooks/useConfigTree";
import { useAppState } from "../providers/AppStateProvider";

const EFFECTIVE_SETTINGS_ROUTE = "virtual:effective-settings";
const EFFECTIVE_MCP_ROUTE = "virtual:effective-mcp";

const MARKDOWN_KINDS: readonly ConfigKind[] = [
  "claude_md",
  "agent",
  "skill",
  "command",
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

interface ConfigSectionProps {
  subRoute: string | null;
  onSubRouteChange: (subRoute: string | null) => void;
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
export function ConfigSection({ subRoute, onSubRouteChange }: ConfigSectionProps) {
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

  const selectedId = useMemo(() => {
    if (!subRoute?.startsWith("node:")) return null;
    return subRoute.slice("node:".length);
  }, [subRoute]);

  const virtualRoute = useMemo<
    null | "effective-settings" | "effective-mcp"
  >(() => {
    if (selectedId === EFFECTIVE_SETTINGS_ROUTE) return "effective-settings";
    if (selectedId === EFFECTIVE_MCP_ROUTE) return "effective-mcp";
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

  const refreshTree = useCallback(async () => {
    try {
      const t = await api.configScan(null);
      setTree(t);
      setLoadError(null);
    } catch (e) {
      setLoadError(String(e));
    }
  }, [setTree]);

  const refreshEditors = useCallback(() => {
    setEditors(null);
    void api
      .configListEditors(true)
      .then(setEditors)
      .catch(() => setEditors([]));
  }, []);

  useEffect(() => {
    void refreshTree();
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
    // Kick off the real-FS watcher; incremental patches arrive via
    // `config-tree-patch` and are applied by useConfigTree.
    void api.configWatchStart(null).catch(() => {
      // Non-fatal — the tree still works via explicit Refresh.
    });
    return () => {
      void api.configWatchStop().catch(() => {});
    };
  }, [refreshTree]);

  // Recovery: orphan watcher patches without a baseline trigger a
  // fresh scan so subsequent patches have a tree to apply to.
  useEffect(() => {
    if (orphanPatchSignal === 0) return;
    void refreshTree();
  }, [orphanPatchSignal, refreshTree]);

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
  useEffect(() => {
    if (!selectedId || selectedId.startsWith("virtual:")) {
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
  }, [selectedId]);

  const startSearch = useCallback(async () => {
    const trimmed = searchQuery.trim();
    if (!trimmed) {
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
    const id = `search-${Date.now()}`;
    setActiveSearchId(id);
    setSearchActive(true);
    setSearchHits([]);
    setSearchSummary(null);
    try {
      await api.configSearchStart(id, {
        text: trimmed,
        regex: searchRegex,
        case_sensitive: false,
      });
    } catch (e) {
      pushToast("error", `Search failed: ${e}`);
      setSearchActive(false);
      setActiveSearchId(null);
    }
  }, [searchQuery, searchRegex, activeSearchId, pushToast]);

  const cancelSearch = useCallback(async () => {
    if (activeSearchId) {
      try {
        await api.configSearchCancel(activeSearchId);
      } catch {
        // ignore
      }
    }
    setActiveSearchId(null);
    setSearchActive(false);
  }, [activeSearchId]);

  const clearSearch = useCallback(async () => {
    await cancelSearch();
    setSearchQuery("");
    setSearchHits([]);
    setSearchSummary(null);
  }, [cancelSearch]);

  useEffect(() => {
    if (!activeSearchId) return;
    let unlisten1: (() => void) | null = null;
    let unlisten2: (() => void) | null = null;
    let cancelled = false;
    void listen<ConfigSearchHitDto>(
      `config-search-hit::${activeSearchId}`,
      (ev) => {
        if (cancelled) return;
        setSearchHits((prev) => [...prev, ev.payload]);
      },
    ).then((u) => {
      if (cancelled) u();
      else unlisten1 = u;
    });
    void listen<ConfigSearchSummaryDto>(
      `config-search-done::${activeSearchId}`,
      (ev) => {
        if (cancelled) return;
        setSearchSummary(ev.payload);
      },
    ).then((u) => {
      if (cancelled) u();
      else unlisten2 = u;
    });
    return () => {
      cancelled = true;
      unlisten1?.();
      unlisten2?.();
    };
  }, [activeSearchId]);

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
      <ScreenHeader
        title="Config"
        subtitle={
          tree
            ? `${tree.scopes.length} scope${tree.scopes.length === 1 ? "" : "s"} · ${tree.cwd}${watcherDirty ? " · updating…" : ""}`
            : "Read-only browser over Claude Code's filesystem artifacts."
        }
        actions={
          <Button
            variant="ghost"
            glyph={NF.refresh}
            onClick={() => void refreshTree()}
            title="Re-scan on-disk artifacts"
          >
            Refresh
          </Button>
        }
      />

      <div
        style={{
          flex: 1,
          display: "grid",
          gridTemplateColumns: "var(--config-tree-width) minmax(0, 1fr)",
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
            >
              <EffectiveRenderer cwd={tree?.cwd ?? null} />
            </EffectiveShell>
          ) : virtualRoute === "effective-mcp" ? (
            <EffectiveShell
              title="Effective MCP"
              subtitle="MCP servers CC would see, per simulation mode."
            >
              <EffectiveMcpRenderer cwd={tree?.cwd ?? null} />
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

type TreeRow =
  | { kind: "scope"; scope: ConfigScopeNodeDto; expanded: boolean }
  | { kind: "virtual-scope"; label: string; expanded: boolean }
  | { kind: "virtual-row"; id: string; label: string }
  | { kind: "file"; file: ConfigFileNodeDto };

const ROW_HEIGHT = 26;

function ConfigTreePane({
  tree,
  loadError,
  selectedId,
  onSelect,
}: {
  tree: ConfigTreeDto | null;
  loadError: string | null;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
}) {
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [effectiveExpanded, setEffectiveExpanded] = useState(true);
  const parentRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!tree) return;
    setExpanded((prev) => {
      const next = { ...prev };
      for (const s of tree.scopes) {
        if (!(s.id in next)) next[s.id] = true;
      }
      return next;
    });
  }, [tree]);

  const rows: TreeRow[] = useMemo(() => {
    const out: TreeRow[] = [];
    out.push({ kind: "virtual-scope", label: "Effective", expanded: effectiveExpanded });
    if (effectiveExpanded) {
      out.push({ kind: "virtual-row", id: EFFECTIVE_SETTINGS_ROUTE, label: "Effective settings" });
      out.push({ kind: "virtual-row", id: EFFECTIVE_MCP_ROUTE, label: "Effective MCP" });
    }
    if (!tree) return out;
    for (const scope of tree.scopes) {
      const open = !!expanded[scope.id];
      out.push({ kind: "scope", scope, expanded: open });
      if (open) {
        for (const f of scope.files) out.push({ kind: "file", file: f });
      }
    }
    return out;
  }, [tree, expanded, effectiveExpanded]);

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

  return (
    <div
      ref={parentRef}
      role="tree"
      aria-label="Config scopes"
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
                onToggle={(id) => {
                  if (id === "virtual") {
                    setEffectiveExpanded((v) => !v);
                  } else {
                    setExpanded((p) => ({ ...p, [id]: !p[id] }));
                  }
                }}
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
  onToggle: (scopeId: string) => void;
}) {
  if (row.kind === "virtual-scope") {
    return (
      <ScopeHeaderButton
        expanded={row.expanded}
        label={row.label}
        count={2}
        onToggle={() => onToggle("virtual")}
      />
    );
  }
  if (row.kind === "virtual-row") {
    const selected = selectedId === row.id;
    return (
      <FileRowButton
        selected={selected}
        label={row.label}
        onSelect={() => onSelect(row.id)}
      />
    );
  }
  if (row.kind === "scope") {
    return (
      <ScopeHeaderButton
        expanded={row.expanded}
        label={row.scope.label}
        count={row.scope.recursive_count}
        onToggle={() => onToggle(row.scope.id)}
      />
    );
  }
  const selected = selectedId === row.file.id;
  const label = row.file.summary_title ?? fileName(row.file.display_path);
  return (
    <FileRowButton
      selected={selected}
      label={label}
      onSelect={() => onSelect(row.file.id)}
      title={row.file.abs_path}
      issuesCount={row.file.issues.length}
      issuesTitle={row.file.issues.join("; ")}
    />
  );
}

function ScopeHeaderButton({
  expanded,
  label,
  count,
  onToggle,
}: {
  expanded: boolean;
  label: string;
  count: number;
  onToggle: () => void;
}) {
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
        padding: "0 var(--sp-12)",
        background: "transparent",
        border: "none",
        cursor: "pointer",
        color: "var(--fg)",
        fontSize: "var(--fs-xs)",
        fontWeight: 600,
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
      <span
        style={{
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-faint)",
          fontWeight: 400,
        }}
      >
        {count}
      </span>
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
}: {
  selected: boolean;
  label: string;
  onSelect: () => void;
  title?: string;
  issuesCount?: number;
  issuesTitle?: string;
}) {
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
        padding: "0 var(--sp-12) 0 var(--sp-28)",
        background: selected ? "var(--bg-active)" : "transparent",
        color: selected ? "var(--accent-ink)" : "var(--fg)",
        border: "none",
        cursor: "pointer",
        fontSize: "var(--fs-xs)",
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
      {issuesCount != null && issuesCount > 0 && (
        <span title={issuesTitle} aria-label={`${issuesCount} issue${issuesCount === 1 ? "" : "s"}`}>
          <Glyph g={NF.warn} color="var(--danger)" />
        </span>
      )}
    </button>
  );
}

function fileName(path: string): string {
  const m = path.match(/([^/\\]+)$/);
  return m ? m[1] : path;
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
      />
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
            <pre
              style={{
                margin: 0,
                padding: "var(--sp-16) var(--sp-20)",
                fontFamily: "var(--mono)",
                fontSize: "var(--fs-xs)",
                whiteSpace: "pre-wrap",
                overflowWrap: "anywhere",
                color: "var(--fg)",
              }}
            >
              {preview.body_utf8}
            </pre>
            {preview.truncated && <TruncationFooter onOpen={onOpen} />}
          </>
        )}
      </div>
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
  children,
}: {
  title: string;
  subtitle: string;
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
