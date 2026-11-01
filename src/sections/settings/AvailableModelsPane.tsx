import { useCallback, useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import type { AvailableModelsState } from "../../api/availableModels";
import { Button } from "../../components/primitives/Button";
import { Input } from "../../components/primitives/Input";
import { IconButton } from "../../components/primitives/IconButton";
import { NF } from "../../icons";
import { toastError } from "../../lib/i18n-error";
import { SettingToggleRow } from "./SettingToggleRow";

// Settings → General → Claude Code behavior.
//
// Edits CC's user-level `availableModels` allowlist and its companion
// `enforceAvailableModels` flag. Both land in one atomic write, so a
// crash can't leave enforcement on beside a list that no longer
// justifies it.
//
// Two things the copy has to be honest about:
//
//   1. `availableModels` alone does NOT constrain the picker's
//      "Default" option, so a user can bypass the list entirely by
//      choosing Default. That's why the enforce switch exists.
//   2. Enforcement is inert with an empty list — CC ignores it, by
//      design, so a bad config can't lock you out of every model. The
//      pane says "set but inert" rather than implying a restriction.
//
// Order is load-bearing and therefore never sorted: with enforcement
// on, Default resolves to the FIRST entry.

/** Entry granularities CC accepts, shown as placeholder guidance. */
const ENTRY_EXAMPLES = "sonnet · claude-opus-4-8 · claude-sonnet-4-5-20250929";

type PushToast = (kind: "info" | "error", text: string) => void;

export function AvailableModelsPane({ pushToast }: { pushToast: PushToast }) {
  const { t } = useTranslation("settings");
  const { state, entries, busy, save } = useAvailableModels(pushToast);
  const enforce = state?.enforce === true;

  return (
    <>
      <div className="settings-row">
        <div style={{ fontSize: "var(--fs-sm)", color: "var(--fg)" }}>
          {t("models.label")}
        </div>
        <div className="settings-row-hint">
          <div style={{ ...NOTE_STYLE, color: "var(--fg-faint)" }}>
            {state == null
              ? t("shared.loading")
              : state.blocks_all
                ? t("models.hintBlocksAll")
                : state.allowlist_restricts
                  ? t("models.hintRestricts")
                  : state.restricted_by_policy
                    ? t("models.hintEmptyPolicy")
                    : t("models.hintEmpty")}
          </div>
          {state?.restricted_by_policy && <PolicyNotes state={state} />}
        </div>

        <AllowlistEditor
          entries={entries}
          disabled={busy || state == null}
          onChange={(next) => void save(next, enforce)}
        />
      </div>

      <SettingToggleRow
        label={t("models.enforceLabel")}
        hint={
          state == null
            ? t("shared.loading")
            : enforce && state.enforce_overridden_by_policy
              ? t("models.enforcePolicy")
              : enforce && !state.enforce_is_effective
                ? t("models.enforceInert", {
                    version: state.enforce_min_cc_version,
                  })
                : t("models.enforceHint", {
                    version: state.enforce_min_cc_version,
                  })
        }
        hintTone={enforce && state != null && !state.enforce_is_effective ? "warn" : "muted"}
        checked={enforce}
        disabled={busy || state == null}
        onChange={(next) => void save(entries, next)}
      />
    </>
  );
}

/** The small-print style shared by the hint and the policy notes. */
const NOTE_STYLE = {
  fontSize: "var(--fs-xs)",
  marginTop: "var(--sp-3)",
  lineHeight: "var(--lh-body)",
} as const;

/**
 * Load the allowlist and write it back. One write at a time: `save`
 * refuses re-entry while a write is in flight.
 */
export function useAvailableModels(pushToast: PushToast) {
  const { t } = useTranslation("settings");
  const [state, setState] = useState<AvailableModelsState | null>(null);
  const [entries, setEntries] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  // A ref, not `busy`: state is read from the render that started the
  // click, so two clicks before React re-renders both see `false`, and
  // their responses can land out of order — leaving the pane showing
  // the older write. The ref flips synchronously.
  const inFlight = useRef(false);

  const refresh = useCallback(async () => {
    try {
      const s = await api.availableModelsState();
      setState(s);
      setEntries(s.entries);
    } catch (e) {
      toastError(pushToast, t("models.loadFailed"), e);
    }
  }, [pushToast, t]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const save = async (nextEntries: string[], nextEnforce: boolean) => {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    try {
      // The backend normalizes (trim, dedupe, order preserved) and
      // returns what actually landed, so the editor shows the file
      // rather than the draft.
      const s = await api.availableModelsSet(nextEntries, nextEnforce);
      setState(s);
      setEntries(s.entries);
      pushToast(
        "info",
        s.allowlist_restricts
          ? t("models.saved", { count: s.entries.length })
          : s.restricted_by_policy
            ? t("models.clearedPolicy")
            : t("models.cleared"),
      );
    } catch (e) {
      toastError(pushToast, t("models.saveFailed"), e);
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  };

  return { state, entries, busy, save };
}

/** What an administrator's policy does to this list, one note each. */
function PolicyNotes({ state }: { state: AvailableModelsState }) {
  const { t } = useTranslation("settings");
  const notes = [
    state.managed_exact_match && t("models.policyExact"),
    state.managed_denied_models.length > 0 &&
      t("models.policyDenied", {
        models: state.managed_denied_models.join(", "),
      }),
  ].filter((n): n is string => typeof n === "string");
  return (
    <>
      {notes.map((note) => (
        <div key={note} style={{ ...NOTE_STYLE, color: "var(--warn)" }}>
          {note}
        </div>
      ))}
    </>
  );
}

/** The ordered list with move / remove controls and the add field. */
function AllowlistEditor({
  entries,
  disabled,
  onChange,
}: {
  entries: string[];
  disabled: boolean;
  onChange: (next: string[]) => void;
}) {
  const { t } = useTranslation("settings");
  const [draft, setDraft] = useState("");
  const inputId = useId();

  const addDraft = () => {
    const trimmed = draft.trim();
    if (!trimmed) return;
    if (!entries.includes(trimmed)) onChange([...entries, trimmed]);
    setDraft("");
  };

  const move = (i: number, delta: number) => {
    const j = i + delta;
    if (j < 0 || j >= entries.length) return;
    const next = [...entries];
    [next[i], next[j]] = [next[j], next[i]];
    onChange(next);
  };

  return (
    <div style={{ display: "grid", gap: "var(--sp-8)" }}>
      {entries.length > 0 && (
        <ul
          style={{
            listStyle: "none",
            margin: 0,
            padding: 0,
            display: "grid",
            gap: "var(--sp-4)",
          }}
        >
          {entries.map((entry, i) => (
            <li
              key={entry}
              style={{
                display: "flex",
                alignItems: "center",
                gap: "var(--sp-6)",
                fontSize: "var(--fs-xs)",
              }}
            >
              <span
                style={{
                  color: "var(--fg-faint)",
                  minWidth: "var(--sp-16)",
                  textAlign: "right",
                }}
              >
                {i + 1}.
              </span>
              <span className="mono selectable" style={{ flex: 1 }}>
                {entry}
              </span>
              <IconButton
                glyph={NF.chevronU}
                size="sm"
                title={t("models.moveUp")}
                aria-label={t("models.moveUpAria", { entry })}
                disabled={disabled || i === 0}
                onClick={() => move(i, -1)}
              />
              <IconButton
                glyph={NF.chevronD}
                size="sm"
                title={t("models.moveDown")}
                aria-label={t("models.moveDownAria", { entry })}
                disabled={disabled || i === entries.length - 1}
                onClick={() => move(i, 1)}
              />
              <IconButton
                glyph={NF.trash}
                size="sm"
                title={t("models.remove")}
                aria-label={t("models.removeAria", { entry })}
                disabled={disabled}
                onClick={() => onChange(entries.filter((_, n) => n !== i))}
              />
            </li>
          ))}
        </ul>
      )}

      <div style={{ display: "flex", gap: "var(--sp-6)" }}>
        <Input
          id={inputId}
          type="text"
          value={draft}
          placeholder={ENTRY_EXAMPLES}
          aria-label={t("models.inputAria")}
          disabled={disabled}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              addDraft();
            }
          }}
          style={{ flex: 1 }}
        />
        <Button
          variant="outline"
          glyph={NF.plus}
          disabled={disabled || !draft.trim()}
          onClick={addDraft}
        >
          {t("models.add")}
        </Button>
      </div>
    </div>
  );
}
