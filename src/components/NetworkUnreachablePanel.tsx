import { openUrl } from "@tauri-apps/plugin-opener";
import { useTranslation } from "react-i18next";
import { Button } from "./primitives/Button";
import { Glyph } from "./primitives/Glyph";
import { IconButton } from "./primitives/IconButton";
import { NF } from "../icons";
import { i18n } from "../lib/i18n";
import type { NetworkDiagnosis } from "../api/service-status";

interface Props {
  diagnosis: NetworkDiagnosis;
  /** Re-run the probe. */
  onRetry: () => void;
  /** Hide for the rest of this session. */
  onDismiss: () => void;
  /** Navigate to Providers section and surface the Add Route
   *  modal. The panel doesn't know about routing — the parent
   *  component (App.tsx) wires this up. */
  onUseProvider: () => void;
  /** Navigate to Settings → Network. Same wiring rationale. */
  onConfigureProxy: () => void;
}

/**
 * First-run network unreachable panel. See
 * `dev-docs/network-detection-panel.md`.
 *
 * Renders when `useNetworkGate` reports `api.anthropic.com` is
 * unreachable. Offers four remediation paths: a provider, proxy
 * config, in-app docs, or dismiss-for-this-session. The
 * underlying app remains usable — sections that don't need the
 * network (Sessions, Memory, Cleanup, Trash) keep working.
 *
 * Per `design.md`'s "one signal per surface", this is the *only*
 * place an Anthropic-unreachable signal surfaces in the shell. The
 * StatusBar dot still reflects service-status tier, but its red
 * state is a different signal (Anthropic is degraded for everyone)
 * vs. this panel's signal (the user's network can't reach
 * Anthropic).
 */
export function NetworkUnreachablePanel({
  diagnosis,
  onRetry,
  onDismiss,
  onUseProvider,
  onConfigureProxy,
}: Props) {
  const { t } = useTranslation("components");
  const copy = copyForDiagnosis(diagnosis);

  return (
    <div
      role="status"
      aria-live="polite"
      style={{
        margin: "var(--sp-12) var(--sp-16) 0",
        padding: "var(--sp-12) var(--sp-16)",
        border: "var(--bw-hair) solid var(--warn)",
        borderRadius: "var(--r-2)",
        background: "color-mix(in oklch, var(--warn) 8%, var(--bg-raised))",
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-10)",
      }}
    >
      {/* Header — glyph + title + close-X */}
      <div
        style={{
          display: "flex",
          alignItems: "flex-start",
          gap: "var(--sp-10)",
        }}
      >
        <Glyph
          g={NF.warn}
          style={{ color: "var(--warn)", flexShrink: 0, marginTop: 2 }}
        />
        <div style={{ flex: 1, display: "flex", flexDirection: "column", gap: "var(--sp-4)" }}>
          <div
            style={{
              fontWeight: 600,
              fontSize: "var(--fs-sm)",
              color: "var(--fg)",
            }}
          >
            {copy.title}
          </div>
          <div
            style={{
              fontSize: "var(--fs-sm)",
              color: "var(--fg-muted)",
              lineHeight: 1.5,
            }}
          >
            {copy.body}
          </div>
        </div>
        <IconButton
          glyph={NF.x}
          title={t("banners.networkDismissTitle")}
          aria-label={t("banners.networkDismissAria")}
          size="sm"
          onClick={onDismiss}
        />
      </div>

      {/* Action row */}
      <div
        style={{
          display: "flex",
          flexWrap: "wrap",
          gap: "var(--sp-8)",
          alignItems: "center",
        }}
      >
        <Button variant="solid" glyph={NF.bolt} onClick={onUseProvider}>
          {t("banners.useProvider")}
        </Button>
        <Button variant="ghost" glyph={NF.globe} onClick={onConfigureProxy}>
          {t("banners.configureProxy")}
        </Button>
        <Button
          variant="ghost"
          glyph={NF.openExternal}
          onClick={openHelpExternal}
        >
          {t("banners.networkHelp")}
        </Button>
        <span style={{ flex: 1 }} />
        <Button variant="subtle" glyph={NF.refresh} onClick={onRetry}>
          {t("banners.retry")}
        </Button>
      </div>
    </div>
  );
}

interface DiagnosisCopy {
  title: string;
  body: string;
}

/**
 * Diagnosis-specific copy. Each branch names what we know and what
 * the user can do — short enough to read in one glance, specific
 * enough to drive the right remediation. The four buttons below the
 * copy are constant; only the title + body shift.
 */
function copyForDiagnosis(d: NetworkDiagnosis): DiagnosisCopy {
  // Plain helper, not a component — reads the global i18n instance.
  // The panel subscribes via `useTranslation`, so a language switch
  // re-renders it and re-invokes this.
  const ns = { ns: "components" } as const;
  switch (d) {
    case "dns_failure":
      return {
        title: i18n.t("banners.dnsTitle", ns),
        body: i18n.t("banners.dnsBody", ns),
      };
    case "timeout":
    case "connection_refused":
      return {
        title: i18n.t("banners.unreachableTitle", ns),
        body: i18n.t("banners.refusedBody", ns),
      };
    case "tls_error":
      return {
        title: i18n.t("banners.tlsTitle", ns),
        body: i18n.t("banners.tlsBody", ns),
      };
    case "http_error":
      return {
        title: i18n.t("banners.httpTitle", ns),
        body: i18n.t("banners.httpBody", ns),
      };
    case "unknown":
    default:
      return {
        title: i18n.t("banners.unreachableTitle", ns),
        body: i18n.t("banners.unknownBody", ns),
      };
  }
}

function openHelpExternal(): void {
  // openUrl is fire-and-forget — failures fall through silently
  // (matches the existing ExternalLink primitive's pattern).
  void openUrl("https://claudepot.com/help/network").catch(() => {});
}
