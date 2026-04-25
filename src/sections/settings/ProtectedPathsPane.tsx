import { useCallback, useEffect, useState } from "react";
import { Icon } from "../../components/Icon";
import { api } from "../../api";
import type { ProtectedPath } from "../../types";

interface Props {
  pushToast: (kind: "info" | "error", text: string) => void;
}

/**
 * Client-side validation for a "protected path" draft. The UI advertises
 * only absolute paths and `~`-prefixed paths as valid — without this
 * guard, anything non-empty would fall through to `protectedPathsAdd`
 * (audit 2026-04-24, T3 H3).
 *
 * Returns `null` when the draft is acceptable, or a human-readable
 * error message otherwise. Rejects:
 *   - empty / whitespace-only strings
 *   - paths containing `..` segments (traversal)
 *   - relative paths (no leading `/`, `~`, drive letter, or `\\`)
 *   - root-only paths: `/`, `~`, `~/`, drive root (`C:\`, `C:/`, or
 *     bare `C:`), UNC root (`\\`)
 *
 * Path-shape checks here mirror the `simplify_windows_path` /
 * `is_absolute` discipline in `.claude/rules/paths.md`: anything that
 * might be a Windows path must be handled explicitly. The host backend
 * still gets the final say on canonical form and conflicts.
 */
export function validateProtectedPath(raw: string): string | null {
  const trimmed = raw.trim();
  if (trimmed.length === 0) {
    return "Path cannot be empty.";
  }
  // Reject anything containing a `..` segment — easy footgun for the
  // protected-paths feature where the goal is to nail down a stable
  // location.
  const hasParentSegment = trimmed
    .split(/[/\\]/)
    .some((seg) => seg === "..");
  if (hasParentSegment) {
    return "Path cannot contain '..' segments.";
  }
  // UNC root only: just leading backslashes with no host/share.
  if (/^\\\\+$/.test(trimmed)) {
    return "Path cannot be a UNC root.";
  }
  // Absolute path shapes — any one of these is sufficient.
  const isUnixAbsolute = trimmed.startsWith("/");
  const isHomeRelative = trimmed === "~" || trimmed.startsWith("~/") || trimmed.startsWith("~\\");
  const isWindowsDrive = /^[A-Za-z]:[\\/]/.test(trimmed);
  // Drive letter without a separator (e.g. `C:`) is not a usable path.
  const isDriveBare = /^[A-Za-z]:$/.test(trimmed);
  const isUncPath = /^\\\\[^\\]+\\[^\\]+/.test(trimmed);
  if (isDriveBare) {
    return "Path cannot be a bare drive letter; specify a directory under the drive.";
  }
  if (!isUnixAbsolute && !isHomeRelative && !isWindowsDrive && !isUncPath) {
    return "Path must be absolute (start with '/', '~', a drive letter, or '\\\\').";
  }
  // Reject filesystem roots: `/`, `~`, `~/`, drive roots like `C:\` /
  // `C:/`. Protecting "everything" is never the user's intent.
  if (trimmed === "/" || trimmed === "~" || trimmed === "~/" || trimmed === "~\\") {
    return "Path cannot be a filesystem root.";
  }
  if (/^[A-Za-z]:[\\/]?$/.test(trimmed)) {
    return "Path cannot be a drive root; specify a directory under the drive.";
  }
  return null;
}

/**
 * Settings → Protected pane.
 *
 * Renders the materialized list (defaults minus tombstones, then user
 * additions). Add/remove/reset are immediate; errors surface inline
 * (per the feedback ladder in `design-patterns.md` — invalid input is
 * a local error, not a toast). Successful reset uses a toast because
 * it's a global state change the user will want to confirm landed.
 */
export function ProtectedPathsPane({ pushToast }: Props) {
  const [items, setItems] = useState<ProtectedPath[]>([]);
  const [loading, setLoading] = useState(true);
  const [draft, setDraft] = useState("");
  const [addError, setAddError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      const list = await api.protectedPathsList();
      setItems(list);
    } catch (e) {
      pushToast("error", `Load failed: ${e}`);
    } finally {
      setLoading(false);
    }
  }, [pushToast]);

  useEffect(() => {
    reload();
  }, [reload]);

  const handleAdd = useCallback(async () => {
    if (busy) return;
    const path = draft.trim();
    const validationError = validateProtectedPath(path);
    if (validationError) {
      setAddError(validationError);
      return;
    }
    setAddError(null);
    setBusy(true);
    try {
      await api.protectedPathsAdd(path);
      setDraft("");
      await reload();
    } catch (err) {
      setAddError(String(err));
    } finally {
      setBusy(false);
    }
  }, [draft, busy, reload]);

  const handleRemove = useCallback(
    async (path: string) => {
      if (busy) return;
      setBusy(true);
      try {
        await api.protectedPathsRemove(path);
        await reload();
      } catch (err) {
        pushToast("error", `Remove failed: ${err}`);
      } finally {
        setBusy(false);
      }
    },
    [busy, reload, pushToast],
  );

  const handleReset = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    try {
      const list = await api.protectedPathsReset();
      setItems(list);
      pushToast("info", "Protected paths reset to defaults.");
    } catch (err) {
      pushToast("error", `Reset failed: ${err}`);
    } finally {
      setBusy(false);
    }
  }, [busy, pushToast]);

  return (
    <section className="settings-group">
      <p className="muted settings-desc">
        Cleaning will not strip <code>~/.claude.json</code> entries or{" "}
        <code>history.jsonl</code> lines for these paths. The CC artifact
        directory under <code>~/.claude/projects/</code> is still removable
        — only sibling state is preserved.
      </p>

      {loading ? (
        <p className="muted small">Loading…</p>
      ) : (
        <ul className="protected-list" role="list" aria-label="Protected paths">
          {items.length === 0 && (
            <li className="protected-row protected-empty">
              <span className="muted small">No protected paths.</span>
            </li>
          )}
          {items.map((p) => (
            <li key={p.path} className="protected-row">
              <code className="protected-path selectable">{p.path}</code>
              <span
                className={`status-badge status-badge-${
                  p.source === "default" ? "ok" : "warn"
                }`}
                title={
                  p.source === "default"
                    ? "Built-in default"
                    : "Added by you"
                }
              >
                {p.source}
              </span>
              <button
                type="button"
                className="btn ghost icon-only sm"
                onClick={() => handleRemove(p.path)}
                disabled={busy}
                aria-label={`Remove ${p.path}`}
                title={`Remove ${p.path}`}
              >
                <Icon name="x" size={12} />
              </button>
            </li>
          ))}
        </ul>
      )}

      <div className="protected-add-form">
        <input
          type="text"
          className="settings-input wide"
          placeholder="/path/to/protect or ~/path"
          value={draft}
          onChange={(e) => {
            setDraft(e.target.value);
            if (addError) setAddError(null);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              handleAdd();
            }
          }}
          disabled={busy}
          aria-invalid={addError != null}
          aria-describedby={addError ? "protected-add-error" : undefined}
        />
        <button
          type="button"
          className="btn primary"
          onClick={handleAdd}
          disabled={busy || draft.trim().length === 0}
          title="Add this path to the protected list"
        >
          Add
        </button>
      </div>
      {addError ? (
        <p id="protected-add-error" className="settings-inline-error" role="alert">
          {addError}
        </p>
      ) : draft.trim().length === 0 ? (
        // Disabled-Add hint per design-principles §3 — surface the
        // reason the primary action is disabled instead of leaving the
        // user to guess.
        <p className="muted small settings-inline-hint">
          Enter an absolute path (or one starting with <code>~</code>) to enable Add.
        </p>
      ) : null}

      <div className="settings-actions">
        <button
          type="button"
          className="btn outline"
          onClick={handleReset}
          disabled={busy || loading}
          title="Discard your additions and removals — restore the built-in defaults"
        >
          Reset to defaults
        </button>
        {(busy || loading) && (
          <span className="muted small settings-inline-hint">
            {loading ? "Loading…" : "Working…"}
          </span>
        )}
      </div>
    </section>
  );
}
