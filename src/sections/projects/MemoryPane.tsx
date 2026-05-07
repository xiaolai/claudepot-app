import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../../api";
import type {
  AutoMemoryStateDto,
  MemoryChange,
  MemoryEnumerate,
  MemoryFileSummary,
} from "../../api/memory";
import { Button } from "../../components/primitives/Button";
import { CopyButton } from "../../components/CopyButton";
import { IconButton } from "../../components/primitives/IconButton";
import { SectionLabel } from "../../components/primitives/SectionLabel";
import { Tag } from "../../components/primitives/Tag";
import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";
import { MarkdownRenderer } from "../config/MarkdownRenderer";
import type { EditorCandidateDto } from "../../types/config";
import { formatRelativeTime, formatSize } from "./format";

interface MemoryPaneProps {
  projectRoot: string;
}

/**
 * Per-project Memory pane. Renders:
 * - The auto-memory toggle (per-project scope: writes settings.local.json).
 * - File list grouped by role (project / auto-memory / global).
 * - Read-only viewer for the selected file.
 * - Change-log timeline filtered to the selected file (or whole project).
 *
 * Refreshes on `memory:changed` events so the user sees CC's writes
 * land in real time when they have the pane open.
 */
export function MemoryPane({ projectRoot }: MemoryPaneProps) {
  const [data, setData] = useState<MemoryEnumerate | null>(null);
  const [autoMem, setAutoMem] = useState<AutoMemoryStateDto | null>(null);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [content, setContent] = useState<string>("");
  const [contentLoading, setContentLoading] = useState(false);
  const [contentError, setContentError] = useState<string | null>(null);
  const [changes, setChanges] = useState<MemoryChange[]>([]);
  const [showDiffFor, setShowDiffFor] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<"rendered" | "raw">("rendered");

  const refresh = useCallback(async () => {
    try {
      const [list, state] = await Promise.all([
        api.memoryListForProject(projectRoot),
        api.autoMemoryState(projectRoot),
      ]);
      setData(list);
      setAutoMem(state);
      setError(null);
      // If nothing selected yet, pick the first project CLAUDE.md or
      // MEMORY.md, whichever appears first.
      setSelectedPath((current) => {
        if (current && list.files.some((f) => f.abs_path === current)) {
          return current;
        }
        return list.files[0]?.abs_path ?? null;
      });
    } catch (e) {
      setError(String(e));
    }
  }, [projectRoot]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Listen for memory:changed events emitted by the watcher and
  // refresh the pane. Audit 2026-05 #9: filter on project_slug AND
  // global events so a sibling project's edits don't trigger a
  // refresh here. Global CLAUDE.md events have project_slug=null and
  // SHOULD refresh because the pane shows the global file too.
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    void listen<{ project_slug?: string | null }>(
      "memory:changed",
      (event) => {
        const eventSlug = event.payload?.project_slug ?? null;
        const ourSlug = data?.anchor.slug ?? null;
        if (eventSlug === null || (ourSlug !== null && eventSlug === ourSlug)) {
          void refresh();
        }
      },
    ).then((u) => {
      unlisten = u;
    });
    return () => {
      unlisten?.();
    };
  }, [refresh, data]);

  // Reload content + change log whenever the selection changes.
  useEffect(() => {
    if (!selectedPath) {
      setContent("");
      setChanges([]);
      return;
    }
    let cancelled = false;
    setContentLoading(true);
    setContentError(null);
    void api
      .memoryReadFile(projectRoot, selectedPath)
      .then((c) => {
        if (!cancelled) setContent(c);
      })
      .catch((e) => {
        if (!cancelled) {
          setContent("");
          setContentError(String(e));
        }
      })
      .finally(() => {
        if (!cancelled) setContentLoading(false);
      });
    void api
      .memoryChangeLog(projectRoot, selectedPath, 50)
      .then((rows) => {
        if (!cancelled) setChanges(rows);
      })
      .catch(() => {
        if (!cancelled) setChanges([]);
      });
    return () => {
      cancelled = true;
    };
  }, [projectRoot, selectedPath]);

  const groups = useMemo(() => groupFiles(data?.files ?? []), [data]);

  if (error) {
    return (
      <section className="memory-pane">
        <SectionLabel>Memory</SectionLabel>
        <p className="muted small">Failed to load memory: {error}</p>
      </section>
    );
  }

  return (
    <section className="memory-pane">
      <header className="memory-pane__header">
        <SectionLabel>Memory</SectionLabel>
        {autoMem && (
          <PerProjectAutoMemoryToggle
            state={autoMem}
            projectRoot={projectRoot}
            onChange={setAutoMem}
          />
        )}
      </header>

      {data && data.files.length === 0 && (
        <p className="muted small memory-pane__empty">
          No memory files yet. CC writes auto-memory to{" "}
          <code className="mono small">{data.anchor.auto_memory_dir}</code>{" "}
          after sessions; project CLAUDE.md is created when you write one.
        </p>
      )}

      {data && data.files.length > 0 && (
        <div className="memory-pane__body">
          <aside className="memory-pane__filelist">
            {groups.map((g) => (
              <div key={g.label} className="memory-pane__filegroup">
                <span className="memory-pane__grouplabel">{g.label}</span>
                <ul role="listbox" aria-label={`${g.label} files`}>
                  {g.files.map((f) => (
                    <li
                      key={f.abs_path}
                      role="option"
                      aria-selected={f.abs_path === selectedPath}
                      tabIndex={0}
                      // Per .claude/rules/path-display.md state B: row
                      // shows the basename only; tooltip discloses the
                      // full path. Canonical copy site is the viewer
                      // header (CopyButton on the selected file).
                      title={f.abs_path}
                      onClick={() => setSelectedPath(f.abs_path)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault();
                          setSelectedPath(f.abs_path);
                        }
                      }}
                      className={
                        f.abs_path === selectedPath
                          ? "memory-pane__fileitem memory-pane__fileitem--selected"
                          : "memory-pane__fileitem"
                      }
                    >
                      <span className="memory-pane__filebasename">
                        {filePillLabel(f, projectRoot)}
                      </span>
                      <FileBadges file={f} />
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </aside>

          <div className="memory-pane__viewer">
            <div className="memory-pane__viewer-header">
              {/* Canonical copy site for the selected memory file —
                  the file list rows defer here per
                  .claude/rules/path-display.md state B. */}
              <span
                className="mono small memory-pane__viewer-path"
                title={selectedPath ?? undefined}
              >
                {selectedPath ? basename(selectedPath) : "—"}
              </span>
              {selectedPath && (
                <span className="memory-pane__viewer-actions">
                  <CopyButton text={selectedPath} />
                  <ViewToggle mode={viewMode} onChange={setViewMode} />
                  <OpenWithMenu absPath={selectedPath} />
                </span>
              )}
            </div>
            {contentLoading && <p className="muted small">Loading…</p>}
            {contentError && (
              <p className="muted small">Read failed: {contentError}</p>
            )}
            {!contentLoading && !contentError && (
              <div className="memory-pane__content">
                {viewMode === "rendered" ? (
                  <MarkdownRenderer body={content} />
                ) : (
                  <pre className="memory-pane__raw">{content}</pre>
                )}
              </div>
            )}

            {changes.length > 0 && (
              <div className="memory-pane__changelog">
                <SectionLabel>{`Change log · ${changes.length}`}</SectionLabel>
                <ul>
                  {changes.map((c) => (
                    <ChangeRow
                      key={c.id}
                      change={c}
                      expanded={showDiffFor === c.id}
                      onToggle={() =>
                        setShowDiffFor((cur) => (cur === c.id ? null : c.id))
                      }
                    />
                  ))}
                </ul>
              </div>
            )}
          </div>
        </div>
      )}
    </section>
  );
}

interface FileGroup {
  label: string;
  files: MemoryFileSummary[];
}

function groupFiles(files: MemoryFileSummary[]): FileGroup[] {
  const project: MemoryFileSummary[] = [];
  const auto: MemoryFileSummary[] = [];
  const global: MemoryFileSummary[] = [];
  for (const f of files) {
    if (
      f.role === "claude_md_project" ||
      f.role === "claude_md_project_local"
    ) {
      project.push(f);
    } else if (f.role === "claude_md_global") {
      global.push(f);
    } else {
      auto.push(f);
    }
  }
  const out: FileGroup[] = [];
  if (project.length) out.push({ label: "Project", files: project });
  if (auto.length) out.push({ label: "Auto-memory", files: auto });
  if (global.length) out.push({ label: "Global", files: global });
  return out;
}

function basename(absPath: string): string {
  const idx = Math.max(absPath.lastIndexOf("/"), absPath.lastIndexOf("\\"));
  return idx >= 0 ? absPath.slice(idx + 1) : absPath;
}

/**
 * Display label for a row in the file list. Falls back to basename for
 * everything except project-local CLAUDE.md, which renders as
 * `.claude/CLAUDE.md` so it's distinguishable from the project-root
 * `CLAUDE.md` in the same group (audit 2026-05 #7).
 */
function filePillLabel(file: MemoryFileSummary, projectRoot: string): string {
  if (file.role === "claude_md_project_local") {
    return ".claude/CLAUDE.md";
  }
  if (file.role === "claude_md_global") {
    return "~/.claude/CLAUDE.md";
  }
  if (
    file.role === "auto_memory_topic" ||
    file.role === "kairos_log"
  ) {
    // For topic + log files, show the path relative to the
    // auto-memory dir if we can; that surfaces sub-folders (e.g.
    // `logs/2026/05/2026-05-04.md`) without the long absolute prefix.
    return relativeMemoryPath(file.abs_path, projectRoot);
  }
  return basename(file.abs_path);
}

function relativeMemoryPath(absPath: string, projectRoot: string): string {
  // Best-effort: split on the slug fragment derived from projectRoot.
  // We don't have the slug client-side; fall back to "memory/<rest>".
  // Use the platform-agnostic basename as the worst-case display.
  const memMarker = "/memory/";
  const memMarkerWin = "\\memory\\";
  const idx = Math.max(
    absPath.indexOf(memMarker),
    absPath.indexOf(memMarkerWin),
  );
  if (idx >= 0) {
    return absPath.slice(idx + 1); // include the leading "memory/"
  }
  void projectRoot;
  return basename(absPath);
}

interface FileBadgesProps {
  file: MemoryFileSummary;
}

function FileBadges({ file }: FileBadgesProps) {
  const recent = isRecent(file.last_change_unix_ns);
  return (
    <span className="memory-pane__filemeta">
      <span className="memory-pane__filesize">{formatSize(file.size_bytes)}</span>
      {file.lines_past_cutoff && file.lines_past_cutoff > 0 ? (
        <Tag tone="warn">+{file.lines_past_cutoff} past 200</Tag>
      ) : null}
      {recent ? <span className="memory-pane__dot" aria-label="recently changed" /> : null}
    </span>
  );
}

function isRecent(ns: number | null): boolean {
  if (ns == null) return false;
  const ageMs = Date.now() - ns / 1_000_000;
  return ageMs < 24 * 3600 * 1000;
}

interface ChangeRowProps {
  change: MemoryChange;
  expanded: boolean;
  onToggle: () => void;
}

function ChangeRow({ change, expanded, onToggle }: ChangeRowProps) {
  const ms = Math.floor(change.detected_at_ns / 1_000_000);
  const summary =
    change.size_before != null && change.size_after != null
      ? `${formatSize(change.size_before)} → ${formatSize(change.size_after)}`
      : change.change_type === "created"
        ? `created · ${formatSize(change.size_after ?? 0)}`
        : `deleted · ${formatSize(change.size_before ?? 0)}`;
  const canExpand = !!change.diff_text;
  return (
    <li className="memory-pane__changerow">
      <button
        type="button"
        className="memory-pane__changetoggle"
        onClick={canExpand ? onToggle : undefined}
        aria-expanded={canExpand ? expanded : undefined}
        disabled={!canExpand}
      >
        <span className="memory-pane__changewhen">
          {formatRelativeTime(ms)}
        </span>
        <span className="memory-pane__changekind">
          {change.change_type}
        </span>
        <span className="memory-pane__changesize">{summary}</span>
        {canExpand ? (
          <Glyph g={expanded ? NF.chevronD : NF.chevronR} />
        ) : (
          <span className="memory-pane__changereason">
            {diffOmitLabel(change.diff_omit_reason)}
          </span>
        )}
      </button>
      {expanded && change.diff_text && (
        <pre className="memory-pane__diff">{change.diff_text}</pre>
      )}
    </li>
  );
}

function diffOmitLabel(reason: MemoryChange["diff_omit_reason"]): string {
  switch (reason) {
    case "too_large":
      return "(too large)";
    case "binary":
      return "(binary)";
    case "endpoint":
      return "";
    case "baseline":
      return "(baseline)";
    case null:
    default:
      return "";
  }
}

interface PerProjectAutoMemoryToggleProps {
  state: AutoMemoryStateDto;
  projectRoot: string;
  onChange: (next: AutoMemoryStateDto) => void;
}

function PerProjectAutoMemoryToggle({
  state,
  projectRoot,
  onChange,
}: PerProjectAutoMemoryToggleProps) {
  const [busy, setBusy] = useState(false);
  const overridden =
    state.decided_by === "env_disable" || state.decided_by === "env_simple";
  const setValue = async (next: boolean | null) => {
    setBusy(true);
    try {
      const updated = await api.autoMemorySet(
        projectRoot,
        "local_project",
        next,
      );
      onChange(updated);
    } catch (e) {
      console.warn("auto-memory set failed", e);
    } finally {
      setBusy(false);
    }
  };

  if (overridden) {
    return (
      <span className="memory-pane__toggle memory-pane__toggle--locked">
        <Tag tone={state.effective ? "neutral" : "warn"}>
          auto-memory {state.effective ? "on" : "off"}
        </Tag>
        <span className="muted small">overridden by env var</span>
      </span>
    );
  }

  const localValue = state.local_project_settings_value;
  const status = state.effective ? "ENABLED" : "DISABLED";
  return (
    <span className="memory-pane__toggle">
      <Tag tone={state.effective ? "neutral" : "warn"}>{status}</Tag>
      <span className="muted small">{state.decided_label}</span>
      <span className="memory-pane__togglebuttons">
        <IconButton
          glyph={NF.check}
          size="sm"
          onClick={() => void setValue(true)}
          disabled={busy || localValue === true}
          aria-pressed={localValue === true}
          title="Enable auto-memory for this project"
          aria-label="Enable auto-memory for this project"
        />
        <IconButton
          glyph={NF.x}
          size="sm"
          onClick={() => void setValue(false)}
          disabled={busy || localValue === false}
          aria-pressed={localValue === false}
          title="Disable auto-memory for this project"
          aria-label="Disable auto-memory for this project"
        />
        {localValue !== null && (
          <IconButton
            glyph={NF.refresh}
            size="sm"
            onClick={() => void setValue(null)}
            disabled={busy}
            title="Clear the project-scope override"
            aria-label="Clear the project-scope override"
          />
        )}
      </span>
      {state.local_settings_gitignored === false && (
        <p className="muted small memory-pane__gitignore-hint">
          ⚠ <code className="mono">.gitignore</code> doesn't cover{" "}
          <code className="mono">settings.local.json</code> — this override
          could be committed by accident.
        </p>
      )}
    </span>
  );
}

interface ViewToggleProps {
  mode: "rendered" | "raw";
  onChange: (next: "rendered" | "raw") => void;
}

function ViewToggle({ mode, onChange }: ViewToggleProps) {
  return (
    <span className="memory-pane__viewtoggle" role="group" aria-label="View mode">
      <button
        type="button"
        className={
          mode === "rendered"
            ? "memory-pane__viewtoggle-btn memory-pane__viewtoggle-btn--active"
            : "memory-pane__viewtoggle-btn"
        }
        onClick={() => onChange("rendered")}
        aria-pressed={mode === "rendered"}
      >
        Rendered
      </button>
      <button
        type="button"
        className={
          mode === "raw"
            ? "memory-pane__viewtoggle-btn memory-pane__viewtoggle-btn--active"
            : "memory-pane__viewtoggle-btn"
        }
        onClick={() => onChange("raw")}
        aria-pressed={mode === "raw"}
      >
        Raw
      </button>
    </span>
  );
}

interface OpenWithMenuProps {
  absPath: string;
}

/**
 * "Open with…" affordance. Reuses the Config section's editor
 * detection: lazily loads the editor list on first open, then offers
 * a dropdown of choices. The default action (icon click) launches
 * the system handler — same posture as Reveal-in-Finder.
 */
function OpenWithMenu({ absPath }: OpenWithMenuProps) {
  const [editors, setEditors] = useState<EditorCandidateDto[] | null>(null);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const wrapperRef = useRef<HTMLSpanElement | null>(null);

  // Lazy load on first open. Editor detection scans $PATH and bundle
  // dirs — not free; defer until the user asks.
  useEffect(() => {
    if (!open || editors !== null) return;
    let cancelled = false;
    void api.configListEditors().then((list) => {
      if (!cancelled) setEditors(list);
    });
    return () => {
      cancelled = true;
    };
  }, [open, editors]);

  // Click-outside dismiss.
  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (!wrapperRef.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  const launch = async (editorId: string | null) => {
    setBusy(true);
    try {
      await api.configOpenInEditorPath(absPath, editorId, null);
      setOpen(false);
    } catch (e) {
      console.warn("open in editor failed", e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <span className="memory-pane__openwith" ref={wrapperRef}>
      <Button
        variant="ghost"
        size="sm"
        glyph={NF.edit}
        onClick={() => setOpen((o) => !o)}
        disabled={busy}
        aria-haspopup="menu"
        aria-expanded={open}
        title="Open this file in an external editor"
      >
        Open with…
      </Button>
      {open && (
        <div role="menu" className="memory-pane__openwith-menu">
          <button
            type="button"
            role="menuitem"
            className="memory-pane__openwith-item"
            onClick={() => void launch(null)}
            disabled={busy}
          >
            System default
          </button>
          {editors === null ? (
            <span className="memory-pane__openwith-loading muted small">
              Detecting editors…
            </span>
          ) : editors.length === 0 ? (
            <span className="memory-pane__openwith-loading muted small">
              No editors detected
            </span>
          ) : (
            editors.map((ed) => (
              <button
                key={ed.id}
                type="button"
                role="menuitem"
                className="memory-pane__openwith-item"
                onClick={() => void launch(ed.id)}
                disabled={busy}
              >
                {ed.label}
              </button>
            ))
          )}
        </div>
      )}
    </span>
  );
}
