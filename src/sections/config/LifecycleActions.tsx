// Disable / Enable / Trash actions rendered into FilePreview's
// PreviewHeader.secondaryActions slot. The classification comes from
// `artifact_classify_path` (read-only, fast); the renderer only ever
// invokes mutating commands when the classification produced a
// trackable triple.

import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";
import { i18n } from "../../lib/i18n";
import { extractMessage, renderError } from "../../lib/i18n-error";
import type { ClassifyPathDto, ConfigFileNodeDto } from "../../types";

export interface LifecycleActionsProps {
  file: ConfigFileNodeDto;
  classification: ClassifyPathDto | null;
  /** Optional project root passed through to the backend so project
   * skills resolve to projectSettings:* not userSettings:*. */
  projectRoot: string | null;
  /** Called after a successful action so the parent can refresh the
   * Config tree, list_disabled, or whichever derived view it owns. */
  onActed: () => void;
  /** Toast hook — same shape as the rest of the app uses. */
  pushToast: (kind: "info" | "error", text: string) => void;
}

/**
 * Renders the right-side action(s) for a single file in the preview
 * header. Three states:
 *
 *   1. Trackable + active   → [Disable] [Trash]
 *   2. Trackable + disabled → [Re-enable] [Trash]
 *   3. Refused              → small inline notice, no buttons
 *
 * Refused paths (plugin/managed/out-of-scope) deliberately render
 * NO buttons, not greyed-out ones — clicking a disabled button is
 * its own UX bug. The notice text comes from the backend's
 * RefuseReason::Display so users see the same wording in toasts and
 * inline.
 */
export function LifecycleActions(props: LifecycleActionsProps) {
  const { classification } = props;
  // Hook-rule compliance: don't conditionally return between hooks.
  // The early return for "not yet classified" / "refused" cases
  // wraps the entire component output; the inner stateful actions
  // live in `<Actions>` which only mounts when we have a Trackable.
  if (!classification) return null;
  if (classification.refused) {
    return <RefusedNotice text={classification.refused} />;
  }
  if (!classification.trackable) return null;
  return <Actions {...props} trackable={classification.trackable} alreadyDisabled={classification.already_disabled} />;
}

function RefusedNotice({ text }: { text: string }) {
  return (
    <div
      role="note"
      title={text}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: "var(--sp-6)",
        padding: "0 var(--sp-8)",
        fontSize: "var(--fs-2xs)",
        color: "var(--fg-faint)",
        letterSpacing: "var(--ls-wide)",
        textTransform: "uppercase",
      }}
    >
      <Glyph g={NF.info} />
      {refusedShort(text)}
    </div>
  );
}

function Actions({
  file,
  trackable,
  alreadyDisabled,
  projectRoot,
  onActed,
  pushToast,
}: {
  file: LifecycleActionsProps["file"];
  trackable: NonNullable<LifecycleActionsProps["classification"]>["trackable"] & object;
  alreadyDisabled: boolean;
  projectRoot: string | null;
  onActed: () => void;
  pushToast: (kind: "info" | "error", text: string) => void;
}) {
  const { t } = useTranslation("config");
  const [busy, setBusy] = useState(false);
  const [confirmTrash, setConfirmTrash] = useState(false);
  const [confirmSuffix, setConfirmSuffix] = useState(false);

  const onDisable = useCallback(async () => {
    setBusy(true);
    try {
      const rec = await api.artifactDisable(
        trackable.scope_root,
        trackable.kind,
        trackable.relative_path,
        "refuse",
        projectRoot,
      );
      pushToast(
        "info",
        t("lifecycle.disabledToast", { kind: rec.kind, name: rec.name }),
      );
      onActed();
    } catch (err) {
      // Raw (untruncated, unredacted) text — this is a control-flow
      // test, not a display string. `renderError` would cap it at 240
      // chars and could push the marker out of range.
      const msg = extractMessage(err);
      if (/already exists/i.test(msg)) {
        // Surface the suffix-retry confirm via a real Modal — see
        // ConfirmDialog mount below.
        setConfirmSuffix(true);
      } else {
        pushToast("error", renderError(err, t("errors.disable")));
      }
    } finally {
      setBusy(false);
    }
  }, [trackable, projectRoot, onActed, pushToast, t]);

  const onDisableWithSuffix = useCallback(async () => {
    setConfirmSuffix(false);
    setBusy(true);
    try {
      const rec = await api.artifactDisable(
        trackable.scope_root,
        trackable.kind,
        trackable.relative_path,
        "suffix",
        projectRoot,
      );
      pushToast(
        "info",
        t("lifecycle.disabledAsToast", { kind: rec.kind, name: rec.name }),
      );
      onActed();
    } catch (err) {
      pushToast("error", renderError(err, t("errors.disable")));
    } finally {
      setBusy(false);
    }
  }, [trackable, projectRoot, onActed, pushToast, t]);

  const onEnable = useCallback(async () => {
    setBusy(true);
    try {
      const rec = await api.artifactEnable(
        trackable.scope_root,
        trackable.kind,
        trackable.relative_path,
        "refuse",
        projectRoot,
      );
      pushToast(
        "info",
        t("lifecycle.reenabledToast", { kind: rec.kind, name: rec.name }),
      );
      onActed();
    } catch (err) {
      pushToast("error", renderError(err, t("errors.reenable")));
    } finally {
      setBusy(false);
    }
  }, [trackable, projectRoot, onActed, pushToast, t]);

  const doTrash = useCallback(async () => {
    setConfirmTrash(false);
    setBusy(true);
    try {
      const entry = await api.artifactTrash(
        trackable.scope_root,
        trackable.kind,
        trackable.relative_path,
        projectRoot,
      );
      pushToast(
        "info",
        t("lifecycle.trashedToast", { id: entry.id.slice(0, 8) }),
      );
      onActed();
    } catch (err) {
      pushToast("error", renderError(err, t("errors.trash")));
    } finally {
      setBusy(false);
    }
  }, [trackable, projectRoot, onActed, pushToast, t]);

  return (
    <>
      <div
        style={{
          display: "inline-flex",
          alignItems: "center",
          gap: "var(--sp-6)",
        }}
      >
        {alreadyDisabled ? (
          <Button
            variant="ghost"
            glyph={NF.refresh}
            onClick={onEnable}
            disabled={busy}
            title={t("lifecycle.reenableTitle")}
          >
            {t("lifecycle.reenable")}
          </Button>
        ) : (
          <Button
            variant="ghost"
            glyph={NF.eyeSlash}
            onClick={onDisable}
            disabled={busy}
            title={t("lifecycle.disableTitle", { path: file.display_path })}
          >
            {t("lifecycle.disable")}
          </Button>
        )}
        <Button
          variant="ghost"
          danger
          glyph={NF.trash}
          onClick={() => setConfirmTrash(true)}
          disabled={busy}
          title={t("lifecycle.trashTitle")}
        >
          {t("lifecycle.trash")}
        </Button>
      </div>
      {confirmTrash && (
        <ConfirmDialog
          title={t("lifecycle.trashConfirmTitle", { kind: trackable.kind })}
          body={t("lifecycle.trashConfirmBody", {
            path: trackable.relative_path,
          })}
          confirmLabel={t("lifecycle.moveToTrash")}
          confirmDanger
          onConfirm={doTrash}
          onCancel={() => setConfirmTrash(false)}
        />
      )}
      {confirmSuffix && (
        <ConfirmDialog
          title={t("lifecycle.suffixTitle")}
          body={t("lifecycle.suffixBody", { kind: trackable.kind })}
          confirmLabel={t("lifecycle.suffixConfirm")}
          onConfirm={onDisableWithSuffix}
          onCancel={() => setConfirmSuffix(false)}
        />
      )}
    </>
  );
}

/** Shorten the backend's RefuseReason for inline display.
 * Backend already includes the explanation — we keep it brief here
 * (the full text is in the title attribute as a tooltip). */
function refusedShort(text: string): string {
  // `text` is the backend's RefuseReason wire string — matched, never
  // rendered; only the short label it maps to is display copy.
  if (text.startsWith("plugin-owned")) {
    return i18n.t("lifecycle.refusedPlugin", { ns: "config" });
  }
  if (text.startsWith("managed by")) {
    return i18n.t("lifecycle.refusedManaged", { ns: "config" });
  }
  if (text.startsWith("outside")) {
    return i18n.t("lifecycle.refusedOutOfScope", { ns: "config" });
  }
  return i18n.t("lifecycle.refusedReadOnly", { ns: "config" });
}
