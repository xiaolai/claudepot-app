import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../../api";
import { Glyph } from "../../components/primitives/Glyph";
import { Tag } from "../../components/primitives/Tag";
import { NF } from "../../icons";
import type { AccountSummary, DesktopIdentity } from "../../types";
import { ActionCard } from "./ActionCard";
import { IdentityPreview } from "./IdentityPreview";

interface Props {
  /** All registered accounts — used to detect "known" vs "stranger". */
  accounts: AccountSummary[];
  /** Disabled while CC-side flows are mid-flight. */
  externallyDisabled: boolean;
  /** Shared adopt action (toasts, refresh, tray rebuild all owned by
   *  the caller). The card only awaits it; any errors surface via the
   *  shared action's own toast, not a duplicate here. Resolves to
   *  `true` iff the bind committed — the card uses this to decide
   *  whether to call `onAdopted` (and close the modal). */
  onAdoptDesktop: (account: AccountSummary) => Promise<boolean>;
  /** Fires after a successful adopt. Parent closes the modal + refreshes. */
  onAdopted: (email: string) => void;
}

/**
 * Desktop import card rendered in AddAccountModal below the CC flow.
 *
 * Three states:
 *   - `new`      — Desktop is signed in as an email not registered
 *                  in Claudepot yet. Disabled with a link to the CC
 *                  tab; the user must register via browser OAuth
 *                  first, then this tab will light up.
 *   - `known`    — Desktop is signed in as a registered email with
 *                  NO snapshot yet. Click to adopt.
 *   - `adopted`  — matching account already has a snapshot. Shows a
 *                  neutral "already bound" confirmation; overwrite
 *                  flows go through the context menu with its confirm
 *                  modal.
 *   - `empty`    — Desktop is signed out or not installed.
 *   - `error`    — probe failed (keychain locked, platform unsupported).
 *
 * Gate: only the authoritative Decrypted probe result is trusted for
 * mutation. A fast-path candidate never lights up the Bind button here.
 */
type Preflight =
  | { kind: "checking" }
  | { kind: "new"; email: string }
  | { kind: "known"; email: string; account: AccountSummary }
  | { kind: "adopted"; email: string; account: AccountSummary }
  | { kind: "empty"; reason: string }
  | { kind: "error"; message: string };

export function DesktopImportCard({
  accounts,
  externallyDisabled,
  onAdoptDesktop,
  onAdopted,
}: Props) {
  const [adopting, setAdopting] = useState(false);
  // Probe result is cached for the lifetime of the card. The live
  // probe runs `/profile` + DPAPI unwrap which is ~1s — rerunning it
  // every time the `accounts` array identity changes (e.g. parent
  // refresh) used to thrash; now it runs once on mount and account-
  // list changes only affect local classification below.
  const [probe, setProbe] = useState<
    | { kind: "loading" }
    | { kind: "ok"; identity: DesktopIdentity }
    | { kind: "failed"; message: string }
  >({ kind: "loading" });

  // The parent closes the modal inside `onAdopted`, which unmounts us.
  // Track mount so the `finally` cleanup doesn't setState on a dead
  // component (React strict-mode warning + leaked closure).
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const id: DesktopIdentity = await api.verifiedDesktopIdentity();
        if (cancelled) return;
        setProbe({ kind: "ok", identity: id });
      } catch (e) {
        if (cancelled) return;
        setProbe({
          kind: "failed",
          message: e instanceof Error ? e.message : String(e),
        });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Pure derivation from the cached probe + current accounts list.
  // Matches the original state machine variant-for-variant — the
  // only change is that re-classifying on account refresh is now a
  // zero-network local computation.
  const preflight: Preflight = useMemo(() => {
    if (probe.kind === "loading") return { kind: "checking" };
    if (probe.kind === "failed") {
      return { kind: "error", message: probe.message };
    }
    const id = probe.identity;
    if (id.error) return { kind: "error", message: id.error };
    if (!id.email) {
      return { kind: "empty", reason: "Claude Desktop is not signed in." };
    }
    // Only Decrypted results are trusted. A fast-path candidate is
    // intentionally NOT offered here (Codex D5-1).
    if (id.probe_method !== "decrypted") {
      return {
        kind: "error",
        message:
          "Live Desktop identity could not be verified (candidate-only). Open Claude Desktop once so Claudepot can decrypt the live token.",
      };
    }
    const match = accounts.find(
      (a) => a.email.toLowerCase() === id.email!.toLowerCase(),
    );
    if (!match) return { kind: "new", email: id.email };
    if (match.desktop_profile_on_disk) {
      return { kind: "adopted", email: id.email, account: match };
    }
    return { kind: "known", email: id.email, account: match };
  }, [probe, accounts]);

  const handleAdopt = async () => {
    if (preflight.kind !== "known") return;
    setAdopting(true);
    try {
      // Route through the shared adopt action — it owns busy keys,
      // toast wording, the post-adopt refresh, and tray rebuild.
      // The action toasts its own errors and resolves with a bool,
      // so we ONLY advance past the modal when the bind actually
      // committed. Otherwise the modal stays open on failure and
      // the user can retry or close manually.
      const ok = await onAdoptDesktop(preflight.account);
      if (ok) onAdopted(preflight.account.email);
    } finally {
      if (mountedRef.current) setAdopting(false);
    }
  };

  const body: {
    subtitle: string;
    cta: string;
    ctaGlyph: typeof NF.desktop;
    disabled: boolean;
    inner?: ReactNode;
    onClick: () => void;
  } = (() => {
    switch (preflight.kind) {
      case "checking":
        return {
          subtitle: "Probing live Claude Desktop identity…",
          cta: "Checking…",
          ctaGlyph: NF.clock,
          disabled: true,
          onClick: () => {},
        };
      case "new":
        return {
          subtitle: `Desktop is signed in as ${preflight.email}, which isn't registered yet. Register it via the CC flow above, then return here.`,
          cta: "Register first",
          ctaGlyph: NF.user,
          disabled: true,
          onClick: () => {},
        };
      case "known":
        return {
          subtitle: `Snapshot the current Desktop session under the stored account.`,
          cta: adopting ? "Binding…" : "Bind",
          ctaGlyph: adopting ? NF.clock : NF.desktop,
          disabled: adopting || externallyDisabled,
          inner: (
            <IdentityPreview
              email={preflight.email}
              subscription={preflight.account.subscription_type}
              orgName={preflight.account.org_name}
            />
          ),
          onClick: () => {
            void handleAdopt();
          },
        };
      case "adopted":
        return {
          subtitle: "Already bound — use Accounts → Set as Desktop to swap in.",
          cta: "Done",
          ctaGlyph: NF.check,
          disabled: true,
          inner: (
            <IdentityPreview
              email={preflight.email}
              subscription={preflight.account.subscription_type}
              orgName={preflight.account.org_name}
              dimmed
              badge={<Tag tone="neutral">already bound</Tag>}
            />
          ),
          onClick: () => {},
        };
      case "empty":
        return {
          subtitle: preflight.reason,
          cta: "Nothing to bind",
          ctaGlyph: NF.info,
          disabled: true,
          onClick: () => {},
        };
      case "error":
        return {
          subtitle: `Couldn't read Desktop session: ${preflight.message}`,
          cta: "Unavailable",
          ctaGlyph: NF.warn,
          disabled: true,
          onClick: () => {},
        };
    }
  })();

  const summary = summaryFor(preflight);

  return (
    <>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-10)",
          margin: "var(--sp-14) 0",
          color: "var(--fg-ghost)",
          fontSize: "var(--fs-xs)",
        }}
        aria-hidden
      >
        <div style={{ flex: 1, height: "var(--bw-hair)", background: "var(--line)" }} />
        <span className="mono-cap">or bind current Claude Desktop session</span>
        <div style={{ flex: 1, height: "var(--bw-hair)", background: "var(--line)" }} />
      </div>

      <div
        style={{
          display: "flex",
          gap: "var(--sp-10)",
          alignItems: "center",
          fontSize: "var(--fs-sm)",
          color: "var(--fg-muted)",
          padding: "var(--sp-10) var(--sp-12)",
          borderRadius: "var(--r-2)",
          background: "var(--bg-sunken)",
          border: "var(--bw-hair) solid var(--line)",
          marginBottom: "var(--sp-16)",
        }}
        aria-live="polite"
      >
        <Glyph g={summary.glyph} style={{ color: summary.tone, flexShrink: 0 }} />
        <div style={{ flex: 1, lineHeight: "var(--lh-body)" }}>{summary.text}</div>
      </div>

      <ActionCard
        glyph={NF.desktop}
        title="Bind current Desktop session"
        subtitle={body.subtitle}
        command="desktop_adopt"
        disabled={body.disabled}
        onClick={body.onClick}
        cta={body.cta}
        ctaGlyph={body.ctaGlyph}
      >
        {body.inner}
      </ActionCard>
    </>
  );
}

function summaryFor(p: Preflight): { glyph: typeof NF.clock; tone: string; text: ReactNode } {
  switch (p.kind) {
    case "checking":
      return {
        glyph: NF.clock,
        tone: "var(--fg-faint)",
        text: "Probing live Desktop identity…",
      };
    case "new":
      return {
        glyph: NF.info,
        tone: "var(--fg-muted)",
        text: (
          <>
            Claude Desktop is signed in as{" "}
            <span style={{ color: "var(--fg)", fontWeight: 600 }}>{p.email}</span> —
            not yet registered.
          </>
        ),
      };
    case "known":
      return {
        glyph: NF.check,
        tone: "var(--ok)",
        text: (
          <>
            Claude Desktop is signed in as{" "}
            <span style={{ color: "var(--fg)", fontWeight: 600 }}>{p.email}</span>.
          </>
        ),
      };
    case "adopted":
      return {
        glyph: NF.check,
        tone: "var(--ok)",
        text: (
          <>
            Snapshot already stored for{" "}
            <span style={{ color: "var(--fg)", fontWeight: 600 }}>{p.email}</span>.
          </>
        ),
      };
    case "empty":
      return { glyph: NF.info, tone: "var(--fg-faint)", text: p.reason };
    case "error":
      return {
        glyph: NF.warn,
        tone: "var(--warn)",
        text: <>Couldn't read Desktop session: {p.message}</>,
      };
  }
}
