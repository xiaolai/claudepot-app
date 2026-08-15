import { useCallback, useEffect, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { CopyButton } from "../../components/CopyButton";
import { i18n } from "../../lib/i18n";
import { IconButton } from "../../components/primitives/IconButton";
import { NF } from "../../icons";
import { SkeletonRows } from "../../components/primitives/Skeleton";
import { api } from "../../api";
import { renderError } from "../../lib/i18n-error";
import type { ProtectedPath } from "../../types";
import { Button } from "../../components/primitives/Button";

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
    return i18n.t("protected.errEmpty", { ns: "settings" });
  }
  // Reject anything containing a `..` segment — easy footgun for the
  // protected-paths feature where the goal is to nail down a stable
  // location.
  const hasParentSegment = trimmed
    .split(/[/\\]/)
    .some((seg) => seg === "..");
  if (hasParentSegment) {
    return i18n.t("protected.errParent", { ns: "settings" });
  }
  // UNC root only: just leading backslashes with no host/share.
  if (/^\\\\+$/.test(trimmed)) {
    return i18n.t("protected.errUncRoot", { ns: "settings" });
  }
  // Absolute path shapes — any one of these is sufficient.
  const isUnixAbsolute = trimmed.startsWith("/");
  const isHomeRelative = trimmed === "~" || trimmed.startsWith("~/") || trimmed.startsWith("~\\");
  const isWindowsDrive = /^[A-Za-z]:[\\/]/.test(trimmed);
  // Drive letter without a separator (e.g. `C:`) is not a usable path.
  const isDriveBare = /^[A-Za-z]:$/.test(trimmed);
  const isUncPath = /^\\\\[^\\]+\\[^\\]+/.test(trimmed);
  if (isDriveBare) {
    return i18n.t("protected.errBareDrive", { ns: "settings" });
  }
  if (!isUnixAbsolute && !isHomeRelative && !isWindowsDrive && !isUncPath) {
    return i18n.t("protected.errNotAbsolute", { ns: "settings" });
  }
  // Reject filesystem roots: `/`, `~`, `~/`, drive roots like `C:\` /
  // `C:/`. Protecting "everything" is never the user's intent.
  if (trimmed === "/" || trimmed === "~" || trimmed === "~/" || trimmed === "~\\") {
    return i18n.t("protected.errFsRoot", { ns: "settings" });
  }
  if (/^[A-Za-z]:[\\/]?$/.test(trimmed)) {
    return i18n.t("protected.errDriveRoot", { ns: "settings" });
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
  const { t } = useTranslation("settings");
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
      pushToast("error", renderError(e, t("protected.loadFailed")));
    } finally {
      setLoading(false);
    }
  }, [pushToast, t]);

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
      setAddError(renderError(err));
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
        pushToast("error", renderError(err, t("protected.removeFailed")));
      } finally {
        setBusy(false);
      }
    },
    [busy, reload, pushToast, t],
  );

  const handleReset = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    try {
      const list = await api.protectedPathsReset();
      setItems(list);
      pushToast("info", t("protected.resetDone"));
    } catch (err) {
      pushToast("error", renderError(err, t("protected.resetFailed")));
    } finally {
      setBusy(false);
    }
  }, [busy, pushToast, t]);

  return (
    <section className="settings-group">
      <p className="muted settings-desc">
        <Trans
          ns="settings"
          i18nKey="protected.desc"
          components={{ code: <code /> }}
        />
      </p>

      {loading ? (
        <SkeletonRows rows={3} />
      ) : (
        <ul
          className="protected-list"
          role="list"
          aria-label={t("protected.listAria")}
        >
          {items.length === 0 && (
            <li className="protected-row protected-empty">
              <span className="muted small">{t("protected.empty")}</span>
            </li>
          )}
          {items.map((p) => (
            <li key={p.path} className="protected-row">
              {/* The path is the row's primary data and this list is
                  its only home, so per .claude/rules/path-display.md
                  state C we ship both the disclosure tooltip and an
                  inline copy affordance. CSS truncates head-first
                  via direction: rtl. */}
              <code
                className="protected-path selectable"
                title={p.path}
              >
                {p.path}
              </code>
              <CopyButton text={p.path} />
              <span
                className={`status-badge status-badge-${
                  p.source === "default" ? "ok" : "warn"
                }`}
                title={
                  p.source === "default"
                    ? t("protected.sourceDefaultTitle")
                    : t("protected.sourceUserTitle")
                }
              >
                {p.source === "default"
                  ? t("protected.sourceDefault")
                  : t("protected.sourceUser")}
              </span>
              <IconButton
                glyph={NF.x}
                size="sm"
                onClick={() => handleRemove(p.path)}
                disabled={busy}
                aria-label={t("protected.removeAria", { path: p.path })}
                title={t("protected.removeAria", { path: p.path })}
              />
            </li>
          ))}
        </ul>
      )}

      <div className="protected-add-form">
        <input
          type="text"
          className="settings-input wide"
          placeholder={t("protected.placeholder")}
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
        <Button
          variant="solid"
          onClick={handleAdd}
          disabled={busy || draft.trim().length === 0}
          title={t("protected.addTitle")}
        >
          {t("protected.add")}
        </Button>
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
          <Trans
            ns="settings"
            i18nKey="protected.addHint"
            components={{ code: <code /> }}
          />
        </p>
      ) : null}

      <div className="settings-actions">
        {/* Was `className="btn outline"` — no `.outline` rule exists in
            any stylesheet, so this silently rendered as a plain `.btn`
            and the author never got the outline treatment they asked
            for. Deleting the dead class would have removed the evidence
            and kept the wrong appearance; the primitive has a real
            `outline` variant, so use it. */}
        <Button
          variant="outline"
          onClick={handleReset}
          disabled={busy || loading}
          title={t("protected.resetTitle")}
        >
          {t("protected.reset")}
        </Button>
        {(busy || loading) && (
          <span className="muted small settings-inline-hint">
            {loading ? t("shared.loading") : t("protected.working")}
          </span>
        )}
      </div>
    </section>
  );
}
