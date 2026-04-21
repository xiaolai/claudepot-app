import { useState } from "react";
import { Button } from "./primitives/Button";
import { Modal } from "./primitives/Modal";
import { api } from "../api";

/**
 * First-run gate for the live Activity feature.
 *
 * Shown once on cold launch when `preferences.activity_consent_seen`
 * is false. Either button dismisses and sets `activity_consent_seen`
 * to true — only Enable also flips `activity_enabled` and starts the
 * runtime. A declining user can opt in later from Settings.
 *
 * The modal deliberately describes exactly what data is read and
 * where it stays — nothing is sent anywhere. The rule from
 * `design.md`: "Privacy is visible" — users should see the scope of
 * the feature, not discover it.
 */

interface Props {
  /** Whether the modal is open. Parent manages visibility based on
   *  `activity_consent_seen`. */
  open: boolean;
  /** Called after the preferences round-trip completes, success or
   *  failure. Parent should refresh its Preferences state. */
  onDismiss: (outcome: "enabled" | "declined" | "error") => void;
}

export function ConsentLiveModal({ open, onDismiss }: Props) {
  const [busy, setBusy] = useState<"enable" | "decline" | null>(null);

  const handleEnable = async () => {
    setBusy("enable");
    try {
      await api.preferencesSetActivity({ enabled: true, consentSeen: true });
      await api.sessionLiveStart();
      onDismiss("enabled");
    } catch {
      onDismiss("error");
    } finally {
      setBusy(null);
    }
  };

  const handleDecline = async () => {
    setBusy("decline");
    try {
      await api.preferencesSetActivity({ enabled: false, consentSeen: true });
      onDismiss("declined");
    } catch {
      onDismiss("error");
    } finally {
      setBusy(null);
    }
  };

  return (
    <Modal
      open={open}
      width="md"
      aria-labelledby="consent-live-title"
      onClose={busy ? undefined : handleDecline}
    >
      <div style={{ padding: "var(--sp-28) var(--sp-32)" }}>
        <h2
          id="consent-live-title"
          style={{
            fontSize: "var(--fs-md)",
            fontWeight: 600,
            margin: 0,
            marginBottom: "var(--sp-16)",
          }}
        >
          Show live Claude sessions?
        </h2>
        <p
          style={{
            fontSize: "var(--fs-sm)",
            color: "var(--fg-muted)",
            margin: 0,
            marginBottom: "var(--sp-12)",
            lineHeight: 1.5,
          }}
        >
          Claudepot can watch the transcript files Claude Code already
          writes to <code>~/.claude/</code> and show you which of your
          sessions are busy, waiting, or idle — at a glance.
        </p>
        <ul
          style={{
            fontSize: "var(--fs-sm)",
            color: "var(--fg-muted)",
            margin: 0,
            marginBottom: "var(--sp-20)",
            paddingLeft: "var(--sp-20)",
            lineHeight: 1.6,
          }}
        >
          <li>Reads files Claude Code already writes to this Mac.</li>
          <li>
            Nothing is sent anywhere — no network, no analytics, on-device.
          </li>
          <li>
            You can exclude specific projects, hide thinking blocks by
            default, or turn the whole feature off in Settings any time.
          </li>
        </ul>
        <div
          style={{
            display: "flex",
            gap: "var(--sp-8)",
            justifyContent: "flex-end",
          }}
        >
          <Button
            variant="ghost"
            disabled={busy !== null}
            onClick={handleDecline}
          >
            {busy === "decline" ? "Saving…" : "Not now"}
          </Button>
          <Button
            variant="solid"
            disabled={busy !== null}
            onClick={handleEnable}
          >
            {busy === "enable" ? "Enabling…" : "Enable activity"}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
