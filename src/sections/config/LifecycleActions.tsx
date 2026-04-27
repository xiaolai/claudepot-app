// Disable / Enable / Trash actions rendered into FilePreview's
// PreviewHeader.secondaryActions slot. The classification comes from
// `artifact_classify_path` (read-only, fast); the renderer only ever
// invokes mutating commands when the classification produced a
// trackable triple.

import { useCallback, useState } from "react";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";
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
      pushToast("info", `Disabled ${rec.kind} "${rec.name}"`);
      onActed();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      if (/already exists/i.test(msg)) {
        // Surface the suffix-retry confirm via a real Modal — see
        // ConfirmDialog mount below.
        setConfirmSuffix(true);
      } else {
        pushToast("error", `Disable failed: ${msg}`);
      }
    } finally {
      setBusy(false);
    }
  }, [trackable, projectRoot, onActed, pushToast]);

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
      pushToast("info", `Disabled ${rec.kind} as "${rec.name}"`);
      onActed();
    } catch (err) {
      pushToast(
        "error",
        `Disable failed: ${err instanceof Error ? err.message : String(err)}`,
      );
    } finally {
      setBusy(false);
    }
  }, [trackable, projectRoot, onActed, pushToast]);

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
      pushToast("info", `Re-enabled ${rec.kind} "${rec.name}"`);
      onActed();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      pushToast("error", `Re-enable failed: ${msg}`);
    } finally {
      setBusy(false);
    }
  }, [trackable, projectRoot, onActed, pushToast]);

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
      pushToast("info", `Moved to trash (${entry.id.slice(0, 8)}…)`);
      onActed();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      pushToast("error", `Trash failed: ${msg}`);
    } finally {
      setBusy(false);
    }
  }, [trackable, projectRoot, onActed, pushToast]);

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
            title="Move back to the active scope"
          >
            Re-enable
          </Button>
        ) : (
          <Button
            variant="ghost"
            glyph={NF.eyeSlash}
            onClick={onDisable}
            disabled={busy}
            title={`Hide from Claude Code without deleting (file ${file.display_path})`}
          >
            Disable
          </Button>
        )}
        <Button
          variant="ghost"
          danger
          glyph={NF.trash}
          onClick={() => setConfirmTrash(true)}
          disabled={busy}
          title="Move to trash (recoverable from Settings → Cleanup for ~30 days)"
        >
          Trash
        </Button>
      </div>
      {confirmTrash && (
        <ConfirmDialog
          title={`Move ${trackable.kind} to trash?`}
          body={`"${trackable.relative_path}" will move to trash. You can restore it from Settings → Cleanup for ~30 days.`}
          confirmLabel="Move to trash"
          confirmDanger
          onConfirm={doTrash}
          onCancel={() => setConfirmTrash(false)}
        />
      )}
      {confirmSuffix && (
        <ConfirmDialog
          title="Name already taken in disabled"
          body={`A disabled ${trackable.kind} with that name already exists. Disable as "<name>-N" instead?`}
          confirmLabel="Use a -N suffix"
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
  if (text.startsWith("plugin-owned")) return "PLUGIN-OWNED";
  if (text.startsWith("managed by")) return "MANAGED";
  if (text.startsWith("outside")) return "OUT OF SCOPE";
  return "READ-ONLY";
}
