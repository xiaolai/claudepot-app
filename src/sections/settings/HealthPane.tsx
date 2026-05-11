import { useCallback, useEffect, useRef, useState } from "react";
import { Button } from "../../components/primitives/Button";
import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";
import { api } from "../../api";
import type {
  DoctorSection,
  DoctorSeverity,
  DoctorSnapshot,
  ParseStatus,
} from "../../api/cc-doctor";

/**
 * Settings → Health pane. Renders the full output of `claude doctor`
 * (scraped via cc_doctor) with a Refresh button and parse-status
 * disclosure.
 *
 * Scope discipline:
 * - Pane *renders* the scrape result; it does NOT re-implement
 *   doctor's logic. CC is the authoritative source.
 * - The action affordances I mentioned in the original plan
 *   (orphan-remove, native-PATH patch, env-var editor) are deferred:
 *   each requires brittle text-parsing of CC's warning strings AND
 *   destructive operations. Ship rendering first; layer affordances
 *   once we see real failure patterns in the parse-failures log.
 * - One action that IS here: surface the dev-side parse-failures
 *   log as a clickable "Open log" link. Trivial to wire; high
 *   leverage when the parser drifts.
 *
 * Failure handling: a snapshot with `parseStatus.kind === "failed"`
 * shows a banner explaining the fallback. Sections still render —
 * the parser may have extracted *some* signal even when it couldn't
 * confirm a clean parse.
 */
interface HealthPaneProps {
  pushToast: (kind: "info" | "error", msg: string) => void;
}

export function HealthPane({ pushToast }: HealthPaneProps) {
  const [snapshot, setSnapshot] = useState<DoctorSnapshot | null>(null);
  const [busy, setBusy] = useState(false);
  const tokenRef = useRef(0);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const load = useCallback(
    async (force: boolean) => {
      const myToken = ++tokenRef.current;
      setBusy(true);
      try {
        const s = await api.ccDoctorSnapshot(force);
        if (!mountedRef.current || myToken !== tokenRef.current) return;
        setSnapshot(s);
      } catch (e) {
        if (!mountedRef.current || myToken !== tokenRef.current) return;
        pushToast("error", `Health refresh failed: ${e}`);
      } finally {
        if (mountedRef.current && myToken === tokenRef.current) {
          setBusy(false);
        }
      }
    },
    [pushToast],
  );

  useEffect(() => {
    void load(false);
  }, [load]);

  const openParseFailuresLog = useCallback(async () => {
    try {
      await api.ccDoctorOpenParseFailuresLog();
    } catch (e) {
      pushToast("error", `Could not open log: ${e}`);
    }
  }, [pushToast]);

  return (
    <section style={paneStyle}>
      <p style={descStyle}>
        Read-only summary of <code style={codeInline}>claude doctor</code>.
        Captured by spawning Claude Code in a pty and parsing its output;
        cached for 60 s. Distinct from Settings → Diagnostics, which shows
        Claudepot’s own self-check.
      </p>

      {snapshot ? (
        <>
          <HeaderRow snapshot={snapshot} busy={busy} onRefresh={() => void load(true)} />
          <ParseStatusBanner status={snapshot.parseStatus} onOpenLog={openParseFailuresLog} />
          {snapshot.sections.length === 0 ? (
            <EmptyState />
          ) : (
            <div style={sectionsListStyle}>
              {snapshot.sections.map((s, i) => (
                <SectionCard key={`${s.title}-${i}`} section={s} />
              ))}
            </div>
          )}
        </>
      ) : (
        <LoadingSkeleton />
      )}
    </section>
  );
}

/* ─── Header (version + install + actions) ─────────────────────── */

function HeaderRow({
  snapshot,
  busy,
  onRefresh,
}: {
  snapshot: DoctorSnapshot;
  busy: boolean;
  onRefresh: () => void;
}) {
  const captured = new Date(snapshot.capturedAtMs);
  return (
    <div style={headerRowStyle}>
      <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-4)" }}>
        <div style={{ display: "flex", alignItems: "center", gap: "var(--sp-10)" }}>
          <SeverityDot severity={snapshot.severity} />
          <span style={{ fontSize: "var(--fs-base)", fontWeight: 600 }}>
            claude {snapshot.ccVersion ?? "version unknown"}
            {snapshot.installType ? ` · ${snapshot.installType}` : ""}
          </span>
        </div>
        {snapshot.installPath ? (
          <code style={{ ...codeInline, color: "var(--fg-muted)" }} title={snapshot.installPath}>
            {snapshot.installPath}
          </code>
        ) : null}
        <span style={{ fontSize: "var(--fs-xs)", color: "var(--fg-faint)" }}>
          Captured {captured.toLocaleTimeString()} · {snapshot.rawBytes.toLocaleString()} B
        </span>
      </div>
      <Button
        variant="ghost"
        size="sm"
        onClick={onRefresh}
        disabled={busy}
        glyph={NF.refresh}
      >
        {busy ? "Refreshing…" : "Refresh"}
      </Button>
    </div>
  );
}

/* ─── Parse-status banner ──────────────────────────────────────── */

function ParseStatusBanner({
  status,
  onOpenLog,
}: {
  status: ParseStatus;
  onOpenLog: () => void;
}) {
  if (status.kind === "ok") return null;
  const isFailed = status.kind === "failed";
  return (
    <div
      style={{
        ...bannerStyle,
        borderColor: isFailed ? "var(--danger)" : "var(--warn)",
        background: isFailed ? "var(--bad-weak)" : "var(--warn-weak)",
      }}
    >
      <div style={{ fontSize: "var(--fs-sm)", fontWeight: 600 }}>
        {isFailed ? "Parser failed" : "Partial parse"}
      </div>
      <div style={{ fontSize: "var(--fs-xs)", color: "var(--fg-muted)", lineHeight: "var(--lh-body)" }}>
        {status.reason}.{" "}
        {isFailed
          ? "The pill in the chrome falls back to the last-known-good snapshot. "
          : "Some sections may be missing. "}
        Raw output is recorded for review.
      </div>
      <Button variant="ghost" size="sm" onClick={onOpenLog} glyph={NF.file}>
        Open parse-failures log
      </Button>
    </div>
  );
}

/* ─── One section block ────────────────────────────────────────── */

function SectionCard({ section }: { section: DoctorSection }) {
  return (
    <article
      style={{
        ...sectionCardStyle,
        borderColor: borderForSeverity(section.severity),
      }}
    >
      <header
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-10)",
          marginBottom: "var(--sp-10)",
        }}
      >
        <SeverityDot severity={section.severity} />
        <h3
          style={{
            margin: 0,
            fontSize: "var(--fs-base)",
            fontWeight: 600,
            color: "var(--fg)",
          }}
        >
          {section.title}
        </h3>
      </header>
      {section.entries.length === 0 ? (
        <div style={{ fontSize: "var(--fs-xs)", color: "var(--fg-faint)" }}>
          (no entries)
        </div>
      ) : (
        <ul style={entriesListStyle}>
          {section.entries.map((e, i) => (
            <li key={i} style={entryRowStyle}>
              <span aria-hidden style={treePrefixStyle}>{e.treePrefix}</span>
              <span style={entryTextStyle}>{e.text}</span>
            </li>
          ))}
        </ul>
      )}
    </article>
  );
}

/* ─── Bits ─────────────────────────────────────────────────────── */

function SeverityDot({ severity }: { severity: DoctorSeverity }) {
  return (
    <span
      aria-hidden
      style={{
        width: "var(--sp-10)",
        height: "var(--sp-10)",
        borderRadius: "var(--r-pill)",
        background: colorForSeverity(severity),
        flexShrink: 0,
      }}
    />
  );
}

function EmptyState() {
  return (
    <div style={emptyStyle}>
      <Glyph g={NF.info} color="var(--fg-faint)" />
      <span style={{ fontSize: "var(--fs-sm)", color: "var(--fg-muted)" }}>
        No sections parsed from the doctor output.
      </span>
    </div>
  );
}

function LoadingSkeleton() {
  return (
    <div style={emptyStyle}>
      <Glyph g={NF.clock} color="var(--fg-faint)" />
      <span style={{ fontSize: "var(--fs-sm)", color: "var(--fg-muted)" }}>
        Running <code style={codeInline}>claude doctor</code>… (first call per
        minute takes 6–10&nbsp;s)
      </span>
    </div>
  );
}

/* ─── Style + token helpers ────────────────────────────────────── */

function colorForSeverity(s: DoctorSeverity): string {
  switch (s) {
    case "healthy":
      return "var(--ok)";
    case "warning":
      return "var(--warn)";
    case "error":
      return "var(--danger)";
  }
}

function borderForSeverity(s: DoctorSeverity): string {
  // Soft tinted border so the severity reads at a glance without
  // turning the whole card into a colored block. Default lines for
  // healthy keeps the unsurprising sections visually quiet.
  switch (s) {
    case "healthy":
      return "var(--line)";
    case "warning":
      return "var(--warn)";
    case "error":
      return "var(--danger)";
  }
}

const paneStyle: React.CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: "var(--sp-16)",
  maxWidth: "var(--content-cap-md)",
};

const descStyle: React.CSSProperties = {
  margin: 0,
  fontSize: "var(--fs-xs)",
  color: "var(--fg-muted)",
  lineHeight: "var(--lh-body)",
};

const headerRowStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  gap: "var(--sp-16)",
  padding: "var(--sp-12) var(--sp-14)",
  border: "var(--bw-hair) solid var(--line)",
  borderRadius: "var(--r-3)",
  background: "var(--bg-raised)",
};

const bannerStyle: React.CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: "var(--sp-6)",
  padding: "var(--sp-10) var(--sp-14)",
  border: "var(--bw-hair) solid",
  borderRadius: "var(--r-3)",
};

const sectionsListStyle: React.CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: "var(--sp-10)",
};

const sectionCardStyle: React.CSSProperties = {
  border: "var(--bw-hair) solid",
  borderRadius: "var(--r-3)",
  padding: "var(--sp-12) var(--sp-14)",
  background: "var(--bg-raised)",
};

const entriesListStyle: React.CSSProperties = {
  margin: 0,
  padding: 0,
  listStyle: "none",
  display: "flex",
  flexDirection: "column",
  gap: "var(--sp-4)",
};

const entryRowStyle: React.CSSProperties = {
  display: "grid",
  gridTemplateColumns: "var(--sp-16) 1fr",
  gap: "var(--sp-8)",
  alignItems: "baseline",
};

const treePrefixStyle: React.CSSProperties = {
  fontFamily: "var(--font)",
  fontSize: "var(--fs-sm)",
  color: "var(--fg-faint)",
  userSelect: "none",
};

const entryTextStyle: React.CSSProperties = {
  fontSize: "var(--fs-sm)",
  color: "var(--fg)",
  fontFamily: "var(--font)",
  userSelect: "text",
  wordBreak: "break-word",
};

const codeInline: React.CSSProperties = {
  fontFamily: "var(--font)",
  fontSize: "var(--fs-xs)",
  background: "var(--bg-sunken)",
  padding: "var(--sp-1) var(--sp-4)",
  borderRadius: "var(--r-1)",
};

const emptyStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: "var(--sp-10)",
  padding: "var(--sp-16)",
  border: "var(--bw-hair) dashed var(--line)",
  borderRadius: "var(--r-3)",
  background: "var(--bg-sunken)",
};
