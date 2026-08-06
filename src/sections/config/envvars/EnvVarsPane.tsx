// Global → Config → Env Variables.
//
// Search first. 308 rows carrying help text and warnings is not a
// scannable list, and one category alone holds 95 of them, so category
// chips cannot be the entry point — they are a grouping *within*
// results. The pane therefore opens on a search field, with filters for
// control type and safety attribute beside it.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useVirtualizer } from "@tanstack/react-virtual";
import { api } from "../../../api";
import { i18n } from "../../../lib/i18n";
import { renderError } from "../../../lib/i18n-error";
import { ConfirmDialog } from "../../../components/ConfirmDialog";
import type { EnvOverview } from "../../../types/ccEnv";
import { commitModeFor, type SecretHandle } from "./choosers";
import { confirmBody } from "./hazards";
import { useEnvVarFilter } from "./useEnvVarFilter";
import { EnvVarsDisclosure } from "./EnvVarsDisclosure";
import { EnvVarsToolbar } from "./EnvVarsToolbar";
import { groupByCategory, itemKey, ItemView } from "./grouping";
import { UndocumentedSection, UnrecognizedBucket } from "./EnvVarsBuckets";

/** Row count above which the list virtualizes. See the call site. */
const VIRTUALIZE_ABOVE = 40;

/**
 * A pending write that needs a confirmation before it lands.
 *
 * The secret arm carries a [`SecretHandle`] rather than the value. The
 * plaintext stays in the password field's DOM node until the moment of the
 * write, so no React state — here or in the chooser — holds a copy across
 * the confirmation dialog, and `wipe()` runs on cancel as well as on
 * success.
 */
type Pending =
  | { kind: "set"; name: string; value: string; title: string; body: string }
  | {
      kind: "secret";
      name: string;
      handle: SecretHandle;
      title: string;
      body: string;
    }
  | { kind: "clear"; name: string; title: string; body: string };

/**
 * Clearing is never live. Claude Code re-applies settings `env`
 * additively — nothing is deleted from a running session — so the old
 * value survives until relaunch, and every clear says so.
 *
 * Resolved per call, never hoisted to a module constant: the confirmation
 * body is built inside a callback, and a constant captured at module load
 * would keep saying it in the language the app booted in.
 */
function relaunchNote(): string {
  return i18n.t("envvars.relaunchNote", { ns: "config" });
}

export function EnvVarsPane() {
  const { t } = useTranslation("config");
  const [data, setData] = useState<EnvOverview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<string>("");
  const [showInfo, setShowInfo] = useState(false);
  const [pending, setPending] = useState<Pending | null>(null);
  const parentRef = useRef<HTMLDivElement | null>(null);
  const { query, setQuery, controls, safety, rows, toggleControl, toggleSafety } =
    useEnvVarFilter(data);

  const load = useCallback(async () => {
    try {
      setData(await api.ccEnvList());
      setError(null);
    } catch (e) {
      setError(renderError(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const apply = useCallback(async (run: () => Promise<EnvOverview>, done: string) => {
    setBusy(true);
    try {
      // The response is the authoritative post-write state, so the row
      // reconciles against the file rather than against its own optimism.
      setData(await run());
      setStatus(done);
      setError(null);
    } catch (e) {
      setError(renderError(e));
      setStatus("");
    } finally {
      setBusy(false);
    }
  }, []);

  const specOf = useCallback(
    (name: string) => data?.documented.find((d) => d.spec.name === name)?.spec,
    [data],
  );

  const onSet = useCallback(
    (name: string, value: string) => {
      const spec = specOf(name);
      if (spec && commitModeFor(spec) !== "immediate") {
        // Secret and hazardous rows get ONE combined confirmation, not
        // two stacked dialogs.
        setPending({
          kind: "set",
          name,
          value,
          title: t("envvars.confirmSetTitle", { name }),
          body: confirmBody(spec.safety.secret, spec.safety.hazards),
        });
        return;
      }
      void apply(
        () => api.ccEnvSet(name, value),
        t("envvars.savedStatus", { name }),
      );
    },
    [apply, specOf, t],
  );

  const onRequestSecret = useCallback(
    (name: string, handle: SecretHandle) => {
      const spec = specOf(name);
      setPending({
        kind: "secret",
        name,
        handle,
        title: t("envvars.confirmSecretTitle", { name }),
        body: confirmBody(true, spec?.safety.hazards ?? []),
      });
    },
    [specOf, t],
  );

  const onClear = useCallback(
    (name: string) => {
      setPending({
        kind: "clear",
        name,
        title: t("envvars.confirmClearTitle", { name }),
        // Says outright that the old value survives until relaunch —
        // clearing is never live.
        body: t("envvars.clearBody", { relaunch: relaunchNote() }),
      });
    },
    [t],
  );

  const runPending = useCallback(() => {
    if (!pending) return;
    const p = pending;
    setPending(null);
    if (p.kind === "set") {
      void apply(
        () => api.ccEnvSet(p.name, p.value),
        t("envvars.savedStatus", { name: p.name }),
      );
    } else if (p.kind === "secret") {
      // Read at the moment of the write, and blank the field whatever
      // happens next. `value` is a function-local const — the same lifetime
      // any bridge argument has.
      const value = p.handle.read();
      p.handle.wipe();
      if (value === null) {
        // The field went away while the dialog was open. Writing `""` here
        // would replace a stored credential with an empty one, which the
        // backend accepts as a legitimate value.
        setStatus(t("envvars.notChanged", { name: p.name }));
        return;
      }
      void apply(
        () => api.ccEnvSet(p.name, value),
        t("envvars.savedStatus", { name: p.name }),
      );
    } else {
      void apply(
        () => api.ccEnvClear(p.name),
        t("envvars.removedStatus", {
          name: p.name,
          relaunch: relaunchNote(),
        }),
      );
    }
  }, [apply, pending, t]);

  /** Cancelling a secret write must destroy the paste too. */
  const cancelPending = useCallback(() => {
    if (pending?.kind === "secret") pending.handle.wipe();
    setPending(null);
  }, [pending]);

  const handlers = useMemo(
    () => ({ onSet, onClear, onRequestSecret }),
    [onSet, onClear, onRequestSecret],
  );

  const items = useMemo(
    () => groupByCategory(rows, data?.categories ?? []),
    [rows, data],
  );

  // Send the scroll container back to the top whenever the filters change.
  // The appendix buckets now live inside this same scroller and are far
  // taller than the results, so a user reading the undocumented names who
  // then types a query would keep their old offset — the matching rows
  // render above the viewport while the count insists there are matches.
  // `scrollTop = 0` rather than `scrollTo({top: 0})`: jsdom implements the
  // property but not the method, so the method form throws in every test
  // that changes a filter.
  useEffect(() => {
    if (parentRef.current) parentRef.current.scrollTop = 0;
  }, [query, controls, safety]);

  // Virtualize only once the list is long enough to be worth it. Below
  // the threshold, absolute positioning and re-measurement are pure
  // overhead — and they cost the user native find-in-page over a result
  // set they can already see. Searching is the primary navigation here,
  // so a handful of hits is the common case, not the rare one.
  const virtualize = rows.length > VIRTUALIZE_ABOVE;
  const virt = useVirtualizer({
    count: virtualize ? items.length : 0,
    getScrollElement: () => parentRef.current,
    // Help and warning text make rows variable-height, so this is only
    // a starting guess; `measureElement` replaces it with the real one.
    estimateSize: () => 132,
    overscan: 6,
  });

  if (error && !data) {
    return (
      <div className="envvar-empty">
        {t("envvars.readFailed", { error })}
      </div>
    );
  }
  if (!data) return <div className="envvar-empty">{t("state.loading")}</div>;

  return (
    <div className="envvar-pane">
      <EnvVarsToolbar
        query={query}
        onQueryChange={setQuery}
        controls={controls}
        safety={safety}
        onToggleControl={toggleControl}
        onToggleSafety={toggleSafety}
        showInfo={showInfo}
        onToggleInfo={() => setShowInfo((v) => !v)}
        onReload={() => void load()}
      />

      {showInfo ? <EnvVarsDisclosure data={data} /> : null}

      {/* Save status is announced rather than only drawn — a change the
          user cannot see confirmed is a change they will make twice. */}
      <p className="envvar-status" role="status" aria-live="polite">
        {error ?? status}
      </p>

      <p className="envvar-count">
        {t("envvars.countLine", {
          shown: rows.length,
          total: data.documented.length,
        })}
      </p>

      {/* The ONE scroll container in this pane. `.envvar-pane` deliberately
          does not scroll — see envvars.css.

          Both appendix buckets live INSIDE this element, as siblings of the
          virtualized wrapper below. That placement is load-bearing twice
          over. As siblings of the *list* they were inflexible boxes beside a
          `flex: 1 1 0%` scroller, which pinned the list at 0px and put every
          row off screen (see scripts/check-envvar-layout.mjs). And they must
          stay OUTSIDE the wrapper, whose children are absolutely positioned
          against a `getTotalSize()` height — nested there they would overlay
          rows or fall outside the measured height entirely. */}
      {/* role="region" + a name, not role="list": this holds documented rows
          AND the two appendix sections, so list semantics would misdescribe
          it. `tabIndex={0}` is required, not decorative — a scrollable box
          that never receives focus cannot be scrolled by keyboard at all. */}
      <div
        className="envvar-list"
        ref={parentRef}
        role="region"
        aria-label={t("envvars.resultsAria")}
        tabIndex={0}
      >
        {virtualize ? (
          <div
            style={{ position: "relative", height: `${virt.getTotalSize()}px` }}
          >
            {virt.getVirtualItems().map((vi) => (
              <div
                key={itemKey(items[vi.index])}
                data-index={vi.index}
                ref={virt.measureElement}
                style={{
                  position: "absolute",
                  top: 0,
                  left: 0,
                  right: 0,
                  transform: `translateY(${vi.start}px)`,
                }}
              >
                <ItemView
                  item={items[vi.index]}
                  busy={busy}
                  handlers={handlers}
                />
              </div>
            ))}
          </div>
        ) : (
          items.map((it) => (
            <ItemView
              key={itemKey(it)}
              item={it}
              busy={busy}
              handlers={handlers}
            />
          ))
        )}

        {/* A filter that matches nothing rendered the count and then blank
            space, which reads as a broken pane rather than an answer. The
            count alone ("0 of 308") is a number, not a statement. */}
        {rows.length === 0 ? (
          <p className="envvar-no-match">{t("envvars.noMatch")}</p>
        ) : null}

        <UnrecognizedBucket data={data} busy={busy} onClear={onClear} />
        <UndocumentedSection data={data} />
      </div>

      {pending ? (
        <ConfirmDialog
          title={pending.title}
          body={pending.body}
          confirmLabel={
            pending.kind === "clear"
              ? t("envvars.remove")
              : t("envvars.apply")
          }
          confirmDanger={pending.kind === "clear"}
          onCancel={cancelPending}
          onConfirm={runPending}
        />
      ) : null}
    </div>
  );
}
