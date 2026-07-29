// Global → Config → Env Variables.
//
// Search first. 308 rows carrying help text and warnings is not a
// scannable list, and one category alone holds 95 of them, so category
// chips cannot be the entry point — they are a grouping *within*
// results. The pane therefore opens on a search field, with filters for
// control type and safety attribute beside it.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { api } from "../../../api";
import { ConfirmDialog } from "../../../components/ConfirmDialog";
import {
  IconButton,
  Input,
  FilterChip,
} from "../../../components/primitives";
import { NF } from "../../../icons";
import type { EnvOverview } from "../../../types/ccEnv";
import { commitModeFor, type SecretHandle } from "./choosers";
import { confirmBody } from "./hazards";
import {
  CONTROLS,
  SAFETY_FILTERS,
  useEnvVarFilter,
} from "./useEnvVarFilter";
import { EnvVarsDisclosure } from "./EnvVarsDisclosure";
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
 */
const RELAUNCH_NOTE =
  "Sessions already running keep the old value until you relaunch Claude Code.";

export function EnvVarsPane() {
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
      setError(String(e));
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
      setError(String(e));
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
          title: `Set ${name}?`,
          body: confirmBody(spec.safety.secret, spec.safety.hazards),
        });
        return;
      }
      void apply(() => api.ccEnvSet(name, value), `Saved ${name}.`);
    },
    [apply, specOf],
  );

  const onRequestSecret = useCallback(
    (name: string, handle: SecretHandle) => {
      const spec = specOf(name);
      setPending({
        kind: "secret",
        name,
        handle,
        title: `Store ${name} in settings.json?`,
        body: confirmBody(true, spec?.safety.hazards ?? []),
      });
    },
    [specOf],
  );

  const onClear = useCallback(
    (name: string) => {
      setPending({
        kind: "clear",
        name,
        title: `Remove ${name}?`,
        body: `The key is removed from settings.json, so Claude Code's own default applies to new sessions. ${RELAUNCH_NOTE}`,
      });
    },
    [],
  );

  const runPending = useCallback(() => {
    if (!pending) return;
    const p = pending;
    setPending(null);
    if (p.kind === "set") {
      void apply(() => api.ccEnvSet(p.name, p.value), `Saved ${p.name}.`);
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
        setStatus(`${p.name} was not changed — the field was no longer there.`);
        return;
      }
      void apply(() => api.ccEnvSet(p.name, value), `Saved ${p.name}.`);
    } else {
      void apply(
        () => api.ccEnvClear(p.name),
        `Removed ${p.name}. ${RELAUNCH_NOTE}`,
      );
    }
  }, [apply, pending]);

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
    return <div className="envvar-empty">Could not read settings: {error}</div>;
  }
  if (!data) return <div className="envvar-empty">Loading…</div>;

  return (
    <div className="envvar-pane">
      <div className="envvar-toolbar">
        <Input
          glyph={NF.search}
          value={query}
          placeholder="Search name or description…"
          aria-label="Search environment variables"
          onChange={(e) => setQuery(e.target.value)}
          style={{ flex: 1 }}
        />
        <IconButton
          glyph={NF.info}
          title="About environment variables"
          aria-label="About environment variables"
          aria-expanded={showInfo}
          onClick={() => setShowInfo((v) => !v)}
        />
        <IconButton
          glyph={NF.refresh}
          title="Reload from disk"
          aria-label="Reload from disk"
          onClick={() => void load()}
        />
      </div>

      <div className="envvar-chips">
        {CONTROLS.map((c) => (
          <FilterChip
            key={c}
            active={controls.has(c)}
            onToggle={() => toggleControl(c)}
          >
            {c}
          </FilterChip>
        ))}
        {SAFETY_FILTERS.map((f) => (
          <FilterChip
            key={f.key}
            active={safety.has(f.key)}
            onToggle={() => toggleSafety(f.key)}
          >
            {f.label}
          </FilterChip>
        ))}
      </div>

      {showInfo ? <EnvVarsDisclosure data={data} /> : null}

      {/* Save status is announced rather than only drawn — a change the
          user cannot see confirmed is a change they will make twice. */}
      <p className="envvar-status" role="status" aria-live="polite">
        {error ?? status}
      </p>

      <p className="envvar-count">
        {rows.length} of {data.documented.length} documented variables
      </p>

      <div className="envvar-list" ref={parentRef}>
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
      </div>

      <UnrecognizedBucket data={data} busy={busy} onClear={onClear} />
      <UndocumentedSection data={data} />

      {pending ? (
        <ConfirmDialog
          title={pending.title}
          body={pending.body}
          confirmLabel={pending.kind === "clear" ? "Remove" : "Apply"}
          confirmDanger={pending.kind === "clear"}
          onCancel={cancelPending}
          onConfirm={runPending}
        />
      ) : null}
    </div>
  );
}
