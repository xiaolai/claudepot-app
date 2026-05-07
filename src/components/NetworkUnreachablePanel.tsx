import { openUrl } from "@tauri-apps/plugin-opener";
import { Button } from "./primitives/Button";
import { Glyph } from "./primitives/Glyph";
import { IconButton } from "./primitives/IconButton";
import { NF } from "../icons";
import type { NetworkDiagnosis } from "../api/service-status";

interface Props {
  diagnosis: NetworkDiagnosis;
  /** Re-run the probe. */
  onRetry: () => void;
  /** Hide for the rest of this session. */
  onDismiss: () => void;
  /** Navigate to Third-parties section and surface the Add Route
   *  modal. The panel doesn't know about routing — the parent
   *  component (App.tsx) wires this up. */
  onUseThirdParty: () => void;
  /** Navigate to Settings → Network. Same wiring rationale. */
  onConfigureProxy: () => void;
}

/**
 * First-run network unreachable panel. See
 * `dev-docs/network-detection-panel.md`.
 *
 * Renders when `useNetworkGate` reports `api.anthropic.com` is
 * unreachable. Offers four remediation paths: third-party LLM,
 * proxy config, in-app docs, or dismiss-for-this-session. The
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
  onUseThirdParty,
  onConfigureProxy,
}: Props) {
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
          title="Dismiss for this session"
          aria-label="Dismiss network panel"
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
        <Button variant="solid" glyph={NF.bolt} onClick={onUseThirdParty}>
          Use a third-party LLM
        </Button>
        <Button variant="ghost" glyph={NF.globe} onClick={onConfigureProxy}>
          Configure proxy
        </Button>
        <Button
          variant="ghost"
          glyph={NF.openExternal}
          onClick={openHelpExternal}
        >
          Network help
        </Button>
        <span style={{ flex: 1 }} />
        <Button variant="subtle" glyph={NF.refresh} onClick={onRetry}>
          Retry
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
  switch (d) {
    case "dns_failure":
      return {
        title: "Claudepot can't resolve api.anthropic.com",
        body: "DNS lookup failed. This is the typical signature of a regional block (mainland China, Russia, Iran), a captive portal that hijacks DNS, or a broken local resolver. Use a third-party LLM to keep working, or fix the resolver below.",
      };
    case "timeout":
    case "connection_refused":
      return {
        title: "Claudepot can't reach api.anthropic.com",
        body: "The path resolves but the connection is being dropped or refused. Often the result of a corporate firewall or a regional block. Use a third-party LLM, or configure a proxy your network allows.",
      };
    case "tls_error":
      return {
        title: "TLS handshake to Anthropic failed",
        body: "The connection started but the encrypted handshake didn't complete. Some networks intercept TLS traffic; others have outdated CA bundles. Configure your proxy if your organization manages one, or use a third-party LLM.",
      };
    case "http_error":
      return {
        title: "Anthropic responded with a service error",
        body: "Reached api.anthropic.com but it isn't returning normal responses. This is usually an Anthropic-side issue — check status.claude.com. Retry in a moment, or use a third-party LLM if you need to keep working.",
      };
    case "unknown":
    default:
      return {
        title: "Claudepot can't reach api.anthropic.com",
        body: "Anthropic's API isn't reachable from this network. This is usually a network or proxy issue, or a region where Anthropic isn't directly reachable.",
      };
  }
}

function openHelpExternal(): void {
  // openUrl is fire-and-forget — failures fall through silently
  // (matches the existing ExternalLink primitive's pattern).
  void openUrl("https://claudepot.com/help/network").catch(() => {});
}
