import { useCallback, useEffect, useId, useState } from "react";
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

export function AvailableModelsPane({
  pushToast,
}: {
  pushToast: (kind: "info" | "error", text: string) => void;
}) {
  const { t } = useTranslation("settings");
  const [state, setState] = useState<AvailableModelsState | null>(null);
  const [entries, setEntries] = useState<string[]>([]);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const inputId = useId();

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
    // `busy` disables the controls, but React renders that a tick after
    // the click, so two fast clicks can both get here and their
    // responses can land out of order — leaving the pane showing the
    // older write. Refuse re-entry outright.
    if (busy) return;
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
        s.restricts_models
          ? t("models.saved", { count: s.entries.length })
          : t("models.cleared"),
      );
    } catch (e) {
      toastError(pushToast, t("models.saveFailed"), e);
    } finally {
      setBusy(false);
    }
  };

  const enforce = state?.enforce === true;

  const addDraft = () => {
    const trimmed = draft.trim();
    if (!trimmed) return;
    if (entries.includes(trimmed)) {
      setDraft("");
      return;
    }
    void save([...entries, trimmed], enforce);
    setDraft("");
  };

  const removeAt = (i: number) =>
    void save(entries.filter((_, n) => n !== i), enforce);

  const move = (i: number, delta: number) => {
    const j = i + delta;
    if (j < 0 || j >= entries.length) return;
    const next = [...entries];
    [next[i], next[j]] = [next[j], next[i]];
    void save(next, enforce);
  };

  return (
    <>
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "var(--settings-label-col) 1fr",
          gap: "var(--sp-16)",
          alignItems: "start",
          padding: "var(--sp-8) 0",
          borderBottom: "var(--bw-hair) solid var(--line)",
        }}
      >
        <div>
          <div style={{ fontSize: "var(--fs-sm)", color: "var(--fg)" }}>
            {t("models.label")}
          </div>
          <div
            style={{
              fontSize: "var(--fs-xs)",
              color: "var(--fg-faint)",
              marginTop: "var(--sp-3)",
              lineHeight: "var(--lh-body)",
            }}
          >
            {state == null
              ? t("shared.loading")
              : state.restricts_models
                ? t("models.hintRestricts")
                : t("models.hintEmpty")}
          </div>
        </div>

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
                    disabled={busy || i === 0}
                    onClick={() => move(i, -1)}
                  />
                  <IconButton
                    glyph={NF.chevronD}
                    size="sm"
                    title={t("models.moveDown")}
                    aria-label={t("models.moveDownAria", { entry })}
                    disabled={busy || i === entries.length - 1}
                    onClick={() => move(i, 1)}
                  />
                  <IconButton
                    glyph={NF.trash}
                    size="sm"
                    title={t("models.remove")}
                    aria-label={t("models.removeAria", { entry })}
                    disabled={busy}
                    onClick={() => removeAt(i)}
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
              disabled={busy || state == null}
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
              disabled={busy || state == null || !draft.trim()}
              onClick={addDraft}
            >
              {t("models.add")}
            </Button>
          </div>
        </div>
      </div>

      <SettingToggleRow
        label={t("models.enforceLabel")}
        hint={
          state == null
            ? t("shared.loading")
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
