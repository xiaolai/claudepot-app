import { useCallback, useEffect, useRef, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { Button } from "../../components/primitives/Button";
import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";
import { api } from "../../api";
import { renderError } from "../../lib/i18n-error";
import { formatNumber, formatTime } from "../../lib/intl";
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
  const { t } = useTranslation("settings");
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
        pushToast("error", renderError(e, t("health.refreshFailed")));
      } finally {
        if (mountedRef.current && myToken === tokenRef.current) {
          setBusy(false);
        }
      }
    },
    [pushToast, t],
  );

  useEffect(() => {
    void load(false);
  }, [load]);

  const openParseFailuresLog = useCallback(async () => {
    try {
      await api.ccDoctorOpenParseFailuresLog();
    } catch (e) {
      pushToast("error", renderError(e, t("health.openLogFailed")));
    }
  }, [pushToast, t]);

  return (
    <section style={paneStyle}>
      <p style={descStyle}>
        <Trans
          ns="settings"
          i18nKey="health.desc"
          components={{ code: <code style={codeInline} /> }}
        />
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
  const { t } = useTranslation("settings");
  const captured = new Date(snapshot.capturedAtMs);
  // Three states for the header:
  // - "measured":  we have a cc_version (from probe or scrape). Show
  //   the real severity dot + version line + path. Normal layout.
  // - "unmeasured": no cc_version AND severity === "unknown". The
  //   pty scrape failed AND the probe didn't find a binary either.
  //   Render in grey with "Couldn't read claude doctor" copy. Refresh
  //   is the primary affordance.
  // The reason this matters: rendering "claude version unknown" with
  // a colored severity dot conflates a metrology failure with a
  // health verdict. The Unknown severity + parse-status banner pair
  // separates those signals so the user knows which to act on.
  const unmeasured =
    snapshot.ccVersion === null && snapshot.severity === "unknown";

  return (
    <div style={headerRowStyle}>
      <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-4)" }}>
        <div style={{ display: "flex", alignItems: "center", gap: "var(--sp-10)" }}>
          <SeverityDot severity={snapshot.severity} />
          <span
            style={{
              fontSize: "var(--fs-base)",
              fontWeight: 600,
              color: unmeasured ? "var(--fg-muted)" : "var(--fg)",
            }}
          >
            {unmeasured
              ? t("health.unmeasured")
              : `claude ${snapshot.ccVersion}${
                  snapshot.installType ? ` · ${snapshot.installType}` : ""
                }`}
          </span>
        </div>
        {snapshot.installPath ? (
          <code style={{ ...codeInline, color: "var(--fg-muted)" }} title={snapshot.installPath}>
            {snapshot.installPath}
          </code>
        ) : null}
        <span style={{ fontSize: "var(--fs-xs)", color: "var(--fg-faint)" }}>
          {unmeasured
            ? t("health.capturedRetry", {
                time: formatTime(captured),
              })
            : t("health.capturedBytes", {
                time: formatTime(captured),
                bytes: formatNumber(snapshot.rawBytes),
              })}
        </span>
      </div>
      <Button
        variant="ghost"
        size="sm"
        onClick={onRefresh}
        disabled={busy}
        glyph={NF.refresh}
      >
        {busy ? t("health.refreshing") : t("shared.refresh")}
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
  const { t } = useTranslation("settings");
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
        {isFailed ? t("health.parserFailed") : t("health.partialParse")}
      </div>
      <div style={{ fontSize: "var(--fs-xs)", color: "var(--fg-muted)", lineHeight: "var(--lh-body)" }}>
        {status.reason}.{" "}
        {isFailed ? t("health.failedNote") : t("health.partialNote")}
        {t("health.rawRecorded")}
      </div>
      <Button variant="ghost" size="sm" onClick={onOpenLog} glyph={NF.file}>
        {t("health.openParseLog")}
      </Button>
    </div>
  );
}

/* ─── One section block ────────────────────────────────────────── */

function SectionCard({ section }: { section: DoctorSection }) {
  const { t } = useTranslation("settings");
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
          {t("health.noEntries")}
        </div>
      ) : (
        <ul style={entriesListStyle}>
          {section.entries.map((e, i) => (
            <li key={i} style={entryRowStyle}>
              <span aria-hidden style={treePrefixStyle}>{e.treePrefix}</span>
              {/* `.selectable` because users paste plugin / install /
                  parse error text into an LLM chat for help. An inline
                  `userSelect: "text"` doesn't work here — see
                  `styles/components/base.css` for the why. */}
              <span className="selectable" style={entryTextStyle}>{e.text}</span>
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
  const { t } = useTranslation("settings");
  return (
    <div style={emptyStyle}>
      <Glyph g={NF.info} color="var(--fg-faint)" />
      <span style={{ fontSize: "var(--fs-sm)", color: "var(--fg-muted)" }}>
        {t("health.noSections")}
      </span>
    </div>
  );
}

function LoadingSkeleton() {
  return (
    <div style={emptyStyle}>
      <Glyph g={NF.clock} color="var(--fg-faint)" />
      <span style={{ fontSize: "var(--fs-sm)", color: "var(--fg-muted)" }}>
        <Trans
          ns="settings"
          i18nKey="health.running"
          components={{ code: <code style={codeInline} /> }}
        />
      </span>
    </div>
  );
}

/* ─── Style + token helpers ────────────────────────────────────── */

function colorForSeverity(s: DoctorSeverity): string {
  switch (s) {
    // Neutral grey, same as the WindowChrome pill's loading state.
    // The intent is "we don't know" — never "this is fine" (green)
    // or "watch out" (yellow). The parse-failure banner carries the
    // actionable detail so this dot only has to convey absence.
    case "unknown":
      return "var(--fg-faint)";
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
    case "unknown":
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
  // user-select handled via the `.selectable` class on the consumer
  // (see the spot in `SectionCard`). React doesn't emit the webkit
  // prefix from `userSelect`, and WKWebView needs it.
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
