import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
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
import { NF } from "../icons";

interface ConfigSectionProps {
  subRoute: string | null;
  onSubRouteChange: (subRoute: string | null) => void;
}

/**
 * Config section — read-only browser over CC's filesystem artifacts.
 *
 * P1 ships the scope-first tree (User / Project / Local / Memory /
 * CLAUDE.md walks) + a minimal preview pane. Later phases layer in
 * secret masking, merge/provenance, effective settings, MCP, plugins,
 * watcher, and the CC-parity harness (see
 * `dev-docs/config-section-plan.md` §15 for the full roadmap).
 *
 * `subRoute` format: `node:<id>` where `<id>` is a FileNode.id. Persists
 * the selection so a return to the section lands on the same row.
 */
export function ConfigSection({ subRoute, onSubRouteChange }: ConfigSectionProps) {
  const [tree, setTree] = useState<ConfigTreeDto | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [editors, setEditors] = useState<EditorCandidateDto[] | null>(null);
  const [defaults, setDefaults] = useState<EditorDefaultsDto | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [preview, setPreview] = useState<ConfigPreviewDto | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);

  const selectedId = useMemo(() => {
    if (!subRoute?.startsWith("node:")) return null;
    return subRoute.slice("node:".length);
  }, [subRoute]);

  const [searchQuery, setSearchQuery] = useState<string>("");
  const [searchRegex, setSearchRegex] = useState<boolean>(false);
  const [searchActive, setSearchActive] = useState<boolean>(false);
  const [searchHits, setSearchHits] = useState<ConfigSearchHitDto[]>([]);
  const [searchSummary, setSearchSummary] =
    useState<ConfigSearchSummaryDto | null>(null);
  const searchIdRef = useRef<string | null>(null);

  const refreshTree = useCallback(async () => {
    try {
      const t = await api.configScan(null);
      setTree(t);
      setLoadError(null);
    } catch (e) {
      setLoadError(String(e));
    }
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
  }, [refreshTree]);

  useEffect(() => {
    if (!toast) return;
    const h = window.setTimeout(() => setToast(null), 4000);
    return () => window.clearTimeout(h);
  }, [toast]);

  // Repair stale subRoute: if the selected id is gone after a rescan,
  // clear it so the preview doesn't hang on a dead target.
  useEffect(() => {
    if (!tree || !selectedId) return;
    const found = tree.scopes.some((s) =>
      s.files.some((f) => f.id === selectedId),
    );
    if (!found) onSubRouteChange(null);
  }, [tree, selectedId, onSubRouteChange]);

  // Pull preview on selection change.
  useEffect(() => {
    if (!selectedId) {
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

  const refreshEditors = useCallback(() => {
    setEditors(null);
    void api
      .configListEditors(true)
      .then(setEditors)
      .catch(() => setEditors([]));
  }, []);

  const startSearch = useCallback(async () => {
    const trimmed = searchQuery.trim();
    if (!trimmed) {
      setSearchActive(false);
      setSearchHits([]);
      setSearchSummary(null);
      return;
    }
    // Cancel any in-flight search.
    if (searchIdRef.current) {
      try {
        await api.configSearchCancel(searchIdRef.current);
      } catch {
        // ignore
      }
    }
    const id = `search-${Date.now()}`;
    searchIdRef.current = id;
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
      setToast(`Search failed: ${e}`);
      setSearchActive(false);
    }
  }, [searchQuery, searchRegex]);

  const cancelSearch = useCallback(async () => {
    if (searchIdRef.current) {
      try {
        await api.configSearchCancel(searchIdRef.current);
      } catch {
        // ignore
      }
    }
    searchIdRef.current = null;
    setSearchActive(false);
  }, []);

  // Subscribe to streaming events for the active search.
  useEffect(() => {
    const id = searchIdRef.current;
    if (!searchActive || !id) return;
    let unlisten1: (() => void) | null = null;
    let unlisten2: (() => void) | null = null;
    let cancelled = false;
    void listen<ConfigSearchHitDto>(
      `config-search-hit::${id}`,
      (ev) => {
        if (cancelled) return;
        setSearchHits((prev) => [...prev, ev.payload]);
      },
    ).then((u) => {
      if (cancelled) {
        u();
      } else {
        unlisten1 = u;
      }
    });
    void listen<ConfigSearchSummaryDto>(
      `config-search-done::${id}`,
      (ev) => {
        if (cancelled) return;
        setSearchSummary(ev.payload);
      },
    ).then((u) => {
      if (cancelled) {
        u();
      } else {
        unlisten2 = u;
      }
    });
    return () => {
      cancelled = true;
      unlisten1?.();
      unlisten2?.();
    };
  }, [searchActive]);

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
        setToast(String(err));
      }
    },
    [selectedFile],
  );

  const openConfigHome = useCallback(
    async (editorId: string | null) => {
      if (!tree?.cwd) return;
      const target = `${tree.cwd}/.claude`;
      try {
        await api.configOpenInEditorPath(target, editorId, null);
      } catch (err) {
        setToast(String(err));
      }
    },
    [tree?.cwd],
  );

  const pickOther = useCallback(async () => {
    try {
      const picked = await openDialog({
        multiple: false,
        title: "Choose editor binary",
      });
      if (typeof picked !== "string") return;
      setToast(`Custom editor paths land with P8: ${picked}`);
    } catch {
      setToast("Could not open file picker");
    }
  }, []);

  const setDefault = useCallback(
    async (kind: ConfigKind | null, editorId: string) => {
      try {
        await api.configSetEditorDefault(kind, editorId);
        const next = await api.configGetEditorDefaults();
        setDefaults(next);
        setToast(
          kind
            ? `Default editor for ${kind} set to ${editorId}`
            : `Fallback editor set to ${editorId}`,
        );
      } catch (err) {
        setToast(String(err));
      }
    },
    [],
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
            ? `${tree.scopes.length} scope${tree.scopes.length === 1 ? "" : "s"} · ${tree.cwd}`
            : "Read-only browser over Claude Code's filesystem artifacts."
        }
        actions={
          <Button
            variant="ghost"
            glyph={NF.refresh}
            onClick={() => {
              void refreshTree();
              refreshEditors();
            }}
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
          gridTemplateColumns: "320px 1fr",
          minHeight: 0,
        }}
      >
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            minHeight: 0,
            background: "var(--bg-sunken)",
          }}
        >
          <div
            style={{
              padding: "var(--sp-8) var(--sp-12)",
              borderBottom: "var(--bw-hair) solid var(--line)",
              display: "flex",
              gap: "var(--sp-6)",
              alignItems: "center",
            }}
          >
            <input
              type="search"
              placeholder="Search contents…"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void startSearch();
                if (e.key === "Escape") {
                  setSearchQuery("");
                  void cancelSearch();
                }
              }}
              aria-label="Search contents"
              style={{
                flex: 1,
                padding: "var(--sp-4) var(--sp-8)",
                fontSize: "var(--fs-xs)",
                background: "var(--bg)",
                border: "var(--bw-hair) solid var(--line)",
                borderRadius: "var(--r-2)",
                color: "var(--fg)",
              }}
            />
            <button
              type="button"
              onClick={() => setSearchRegex((v) => !v)}
              title="Toggle regex mode"
              aria-pressed={searchRegex}
              className="pm-focus"
              style={{
                padding: "0 var(--sp-6)",
                fontSize: "var(--fs-2xs)",
                fontFamily: "var(--mono)",
                background: searchRegex
                  ? "var(--accent-soft)"
                  : "transparent",
                color: searchRegex ? "var(--accent-ink)" : "var(--fg-muted)",
                border: "var(--bw-hair) solid var(--line)",
                borderRadius: "var(--r-2)",
                cursor: "pointer",
              }}
            >
              .*
            </button>
            {searchActive && (
              <button
                type="button"
                onClick={() => void cancelSearch()}
                title="Cancel search"
                className="pm-focus"
                style={{
                  padding: "0 var(--sp-6)",
                  fontSize: "var(--fs-2xs)",
                  background: "transparent",
                  color: "var(--fg-muted)",
                  border: "var(--bw-hair) solid var(--line)",
                  borderRadius: "var(--r-2)",
                  cursor: "pointer",
                }}
              >
                ×
              </button>
            )}
          </div>

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
            borderLeft: "var(--bw-hair) solid var(--line)",
          }}
        >
          {selectedFile ? (
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
              cwd={tree?.cwd ?? null}
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

      {toast && (
        <div
          role="status"
          aria-live="polite"
          style={{
            position: "fixed",
            bottom: "var(--sp-24)",
            right: "var(--sp-24)",
            padding: "var(--sp-8) var(--sp-12)",
            background: "var(--bg-elev)",
            border: "var(--bw-hair) solid var(--line-strong)",
            borderRadius: "var(--r-2)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg)",
            boxShadow: "var(--shadow-md)",
            maxWidth: 360,
          }}
        >
          {toast}
        </div>
      )}
    </div>
  );
}

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
    <nav
      role="tree"
      aria-label="Config scopes"
      style={{
        flex: 1,
        minHeight: 0,
        overflowY: "auto",
        padding: "var(--sp-8) 0",
      }}
    >
      {tree.scopes.map((s) => (
        <ScopeRow
          key={s.id}
          scope={s}
          expanded={!!expanded[s.id]}
          selectedId={selectedId}
          onToggle={() =>
            setExpanded((p) => ({ ...p, [s.id]: !p[s.id] }))
          }
          onSelect={onSelect}
        />
      ))}
    </nav>
  );
}

function ScopeRow({
  scope,
  expanded,
  selectedId,
  onToggle,
  onSelect,
}: {
  scope: ConfigScopeNodeDto;
  expanded: boolean;
  selectedId: string | null;
  onToggle: () => void;
  onSelect: (id: string | null) => void;
}) {
  return (
    <div role="treeitem" aria-expanded={expanded}>
      <button
        type="button"
        onClick={onToggle}
        className="pm-focus"
        style={{
          display: "flex",
          alignItems: "center",
          width: "100%",
          gap: "var(--sp-6)",
          padding: "var(--sp-4) var(--sp-12)",
          background: "transparent",
          border: "none",
          cursor: "pointer",
          color: "var(--fg)",
          fontSize: "var(--fs-xs)",
          fontWeight: 600,
          textAlign: "left",
        }}
      >
        <span style={{ width: 12, display: "inline-block", textAlign: "center" }}>
          {expanded ? "▾" : "▸"}
        </span>
        <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis" }}>
          {scope.label}
        </span>
        <span
          style={{
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-faint)",
            fontWeight: 400,
          }}
        >
          {scope.recursive_count}
        </span>
      </button>
      {expanded && (
        <ul role="group" style={{ listStyle: "none", padding: 0, margin: 0 }}>
          {scope.files.map((f) => (
            <li key={f.id}>
              <button
                type="button"
                role="treeitem"
                aria-selected={selectedId === f.id}
                onClick={() => onSelect(f.id)}
                className="pm-focus"
                style={{
                  display: "flex",
                  alignItems: "center",
                  width: "100%",
                  gap: "var(--sp-6)",
                  padding: "var(--sp-3) var(--sp-12) var(--sp-3) var(--sp-28)",
                  background:
                    selectedId === f.id ? "var(--bg-active)" : "transparent",
                  color:
                    selectedId === f.id ? "var(--accent-ink)" : "var(--fg)",
                  border: "none",
                  cursor: "pointer",
                  fontSize: "var(--fs-xs)",
                  textAlign: "left",
                }}
                title={f.abs_path}
              >
                <span
                  style={{
                    flex: 1,
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                  }}
                >
                  {f.summary_title ?? fileName(f.display_path)}
                </span>
                {f.issues.length > 0 && (
                  <span
                    title={f.issues.join("; ")}
                    style={{
                      color: "var(--danger)",
                      fontSize: "var(--fs-2xs)",
                    }}
                  >
                    ⚠
                  </span>
                )}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function fileName(path: string): string {
  const m = path.match(/([^/\\]+)$/);
  return m ? m[1] : path;
}

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
        kind={file.kind as ConfigKind}
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
          padding: "var(--sp-16) var(--sp-20)",
          fontFamily: "var(--mono)",
          fontSize: "var(--fs-xs)",
          whiteSpace: "pre-wrap",
          color: "var(--fg)",
          background: "var(--bg)",
        }}
      >
        {previewError ? (
          <div style={{ color: "var(--danger)" }}>Preview failed: {previewError}</div>
        ) : preview ? (
          <>
            {preview.body_utf8}
            {preview.truncated && (
              <div
                style={{
                  marginTop: "var(--sp-12)",
                  color: "var(--fg-faint)",
                  fontStyle: "italic",
                }}
              >
                … preview truncated at 256 KB. Open in editor to see full file.
              </div>
            )}
          </>
        ) : (
          <div style={{ color: "var(--fg-faint)" }}>Loading…</div>
        )}
      </div>
    </div>
  );
}

function ConfigHomePane({
  cwd,
  editors,
  defaults,
  onOpen,
  onPickOther,
  onSetDefault,
  onRefreshEditors,
}: {
  cwd: string | null;
  editors: EditorCandidateDto[] | null;
  defaults: EditorDefaultsDto | null;
  onOpen: (editorId: string | null) => void;
  onPickOther: () => void;
  onSetDefault: (kind: ConfigKind | null, editorId: string) => void;
  onRefreshEditors: () => void;
}) {
  const claudeDir = cwd ? `${cwd}/.claude` : null;
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
        subtitle="Pick a file on the left to preview it."
        path={claudeDir}
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
        Select an artifact from the tree to see its contents, or open the
        whole <code>.claude/</code> folder in your editor.
      </div>
    </div>
  );
}

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
          ? `${summary.total_hits} ${summary.capped ? "(capped) " : ""}hit${summary.total_hits === 1 ? "" : "s"}${summary.skipped_large > 0 ? ` · ${summary.skipped_large} file${summary.skipped_large === 1 ? "" : "s"} skipped (>2MB)` : ""}${summary.cancelled ? " · cancelled" : ""}`
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
              maxHeight: "3.6em",
            }}
          >
            {hit.snippet}
          </pre>
        </button>
      ))}
    </div>
  );
}
