import { useEffect, useId, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { api } from "../../api";
import {
  WINDOW_LABELS,
  newRule,
  type RotationDryRun,
  type RotationRule,
  type SelectorKind,
  type WindowId,
} from "../../api/rotation";
import { Button } from "../../components/primitives/Button";
import {
  Modal,
  ModalBody,
  ModalFooter,
  ModalHeader,
} from "../../components/primitives/Modal";
import { NF } from "../../icons";
import { renderError } from "../../lib/i18n-error";
import {
  CandidateChecklist,
  Field,
  ModeRadio,
  clampPct,
  inputStyle,
  selectStyle,
  suggestId,
} from "./RotationRuleModal.fields";

interface Props {
  existing: RotationRule | null;
  existingIds: string[];
  candidateEmails: string[];
  onClose: () => void;
  onSave: (rule: RotationRule) => Promise<void>;
}

const WINDOW_IDS: WindowId[] = [
  "five_hour",
  "seven_day",
  "seven_day_opus",
  "seven_day_sonnet",
];

export function RotationRuleModal({
  existing,
  existingIds,
  candidateEmails,
  onClose,
  onSave,
}: Props) {
  const { t } = useTranslation("settings");
  const titleId = useId();
  const isEdit = existing != null;

  const SELECTOR_KINDS: { value: SelectorKind; label: string; hint: string }[] = [
    {
      value: "least_used",
      label: t("rotationModal.selector.leastUsed"),
      hint: t("rotationModal.selector.leastUsedHint"),
    },
    {
      value: "round_robin",
      label: t("rotationModal.selector.roundRobin"),
      hint: t("rotationModal.selector.roundRobinHint"),
    },
    {
      value: "explicit",
      label: t("rotationModal.selector.explicit"),
      hint: t("rotationModal.selector.explicitHint"),
    },
  ];

  const [rule, setRule] = useState<RotationRule>(() =>
    existing ?? newRule(suggestId(existingIds), candidateEmails),
  );
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [dry, setDry] = useState<RotationDryRun | null>(null);
  const [dryLoading, setDryLoading] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(false);

  // Keep the selector's window in sync with the trigger window when
  // the user picks `least_used`. The threshold logic compares against
  // the trigger window; mismatch yields confusing rules.
  useEffect(() => {
    if (rule.action.selector.kind === "least_used") {
      if (
        rule.trigger.kind === "utilization_threshold" &&
        rule.trigger.window &&
        rule.action.selector.window !== rule.trigger.window
      ) {
        setRule((r) => ({
          ...r,
          action: {
            ...r.action,
            selector: { ...r.action.selector, window: r.trigger.window },
          },
        }));
      }
    }
  }, [rule.trigger.window, rule.trigger.kind, rule.action.selector.kind, rule.action.selector.window]);

  const candidateOptions = useMemo(
    () => candidateEmails.filter((e) => e.length > 0),
    [candidateEmails],
  );

  const setTriggerWindow = (w: WindowId) => {
    setRule((r) => ({ ...r, trigger: { ...r.trigger, window: w } }));
  };
  const setTriggerPct = (pct: number) => {
    setRule((r) => ({ ...r, trigger: { ...r.trigger, pct } }));
  };
  const setSelectorKind = (kind: SelectorKind) => {
    setRule((r) => ({
      ...r,
      action: { ...r.action, selector: { ...r.action.selector, kind } },
    }));
  };
  const toggleCandidate = (email: string) => {
    setRule((r) => {
      const has = r.action.selector.candidates.includes(email);
      const next = has
        ? r.action.selector.candidates.filter((c) => c !== email)
        : [...r.action.selector.candidates, email];
      return {
        ...r,
        action: {
          ...r.action,
          selector: { ...r.action.selector, candidates: next },
        },
      };
    });
  };
  const setExplicitEmail = (email: string) => {
    setRule((r) => ({
      ...r,
      action: { ...r.action, selector: { ...r.action.selector, email } },
    }));
  };

  const handleSave = async () => {
    setError(null);
    setSaving(true);
    try {
      await api.rotationRuleValidate(rule);
      if (
        !isEdit &&
        existingIds.includes(rule.id.trim())
      ) {
        throw new Error(t("rotationModal.idExists", { id: rule.id }));
      }
      await onSave(rule);
    } catch (e) {
      setError(renderError(e));
    } finally {
      setSaving(false);
    }
  };

  const runDryRun = async () => {
    setDry(null);
    setDryLoading(true);
    try {
      const r = await api.rotationDryRun(rule);
      setDry(r);
    } catch (e) {
      setDry({
        wouldFire: false,
        targetEmail: null,
        reason: t("rotationModal.dryRunFailed", { error: renderError(e) }),
      });
    } finally {
      setDryLoading(false);
    }
  };

  return (
    <Modal open onClose={onClose} aria-labelledby={titleId} width="lg">
      <ModalHeader
        glyph={NF.refresh}
        title={
          isEdit
            ? t("rotationModal.titleEdit", { id: existing.id })
            : t("rotationModal.titleNew")
        }
        id={titleId}
        onClose={onClose}
      />
      <ModalBody>
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-16)" }}>
          {/* Rule id */}
          <Field
            label={t("rotationModal.ruleId.label")}
            hint={t("rotationModal.ruleId.hint")}
          >
            <input
              type="text"
              value={rule.id}
              disabled={isEdit}
              onChange={(e) => setRule((r) => ({ ...r, id: e.target.value }))}
              style={inputStyle()}
            />
          </Field>

          {/* Trigger */}
          <Field
            label={t("rotationModal.fireWhen.label")}
            hint={t("rotationModal.fireWhen.hint")}
          >
            <div style={{ display: "flex", gap: "var(--sp-8)", flexWrap: "wrap" }}>
              <select
                value={rule.trigger.window}
                onChange={(e) => setTriggerWindow(e.target.value as WindowId)}
                style={selectStyle()}
              >
                {WINDOW_IDS.map((w) => (
                  <option key={w} value={w}>
                    {WINDOW_LABELS[w]}
                  </option>
                ))}
              </select>
              <span style={{ alignSelf: "center", fontSize: "var(--fs-sm)" }}>≥</span>
              <input
                type="number"
                min={1}
                max={100}
                value={rule.trigger.pct}
                onChange={(e) =>
                  setTriggerPct(clampPct(Number(e.target.value)))
                }
                style={{ ...inputStyle(), width: "var(--sp-80)" }}
              />
              <span style={{ alignSelf: "center", fontSize: "var(--fs-sm)" }}>%</span>
            </div>
          </Field>

          {/* Selector */}
          <Field label={t("rotationModal.rotateTo")} hint="">
            <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-8)" }}>
              <select
                value={rule.action.selector.kind}
                onChange={(e) => setSelectorKind(e.target.value as SelectorKind)}
                style={selectStyle()}
              >
                {SELECTOR_KINDS.map((s) => (
                  <option key={s.value} value={s.value}>
                    {s.label}
                  </option>
                ))}
              </select>
              <small style={{ color: "var(--fg-muted)", fontSize: "var(--fs-xs)" }}>
                {SELECTOR_KINDS.find((s) => s.value === rule.action.selector.kind)?.hint}
              </small>
              {rule.action.selector.kind === "explicit" ? (
                <select
                  value={rule.action.selector.email}
                  onChange={(e) => setExplicitEmail(e.target.value)}
                  style={selectStyle()}
                >
                  <option value="">{t("rotationModal.pickAccount")}</option>
                  {candidateOptions.map((email) => (
                    <option key={email} value={email}>
                      {email}
                    </option>
                  ))}
                </select>
              ) : (
                <CandidateChecklist
                  options={candidateOptions}
                  selected={rule.action.selector.candidates}
                  onToggle={toggleCandidate}
                />
              )}
            </div>
          </Field>

          {/* Mode */}
          <Field
            label={t("rotationModal.mode.label")}
            hint={t("rotationModal.mode.hint")}
          >
            <div style={{ display: "flex", gap: "var(--sp-12)" }}>
              <ModeRadio
                value="confirm"
                current={rule.mode}
                label={t("rotationModal.mode.confirm")}
                onChange={(v) => setRule((r) => ({ ...r, mode: v }))}
              />
              <ModeRadio
                value="auto"
                current={rule.mode}
                label={t("rotationModal.mode.auto")}
                onChange={(v) => setRule((r) => ({ ...r, mode: v }))}
              />
            </div>
          </Field>

          {/* Advanced (guards) */}
          <details
            open={advancedOpen}
            onToggle={(e) =>
              setAdvancedOpen((e.currentTarget as HTMLDetailsElement).open)
            }
          >
            <summary
              style={{
                cursor: "pointer",
                fontSize: "var(--fs-sm)",
                color: "var(--fg-muted)",
              }}
            >
              {t("rotationModal.advanced")}
            </summary>
            <div
              style={{
                display: "flex",
                flexDirection: "column",
                gap: "var(--sp-12)",
                paddingTop: "var(--sp-12)",
              }}
            >
              <Field
                label={t("rotationModal.minInterval.label")}
                hint={t("rotationModal.minInterval.hint")}
              >
                <input
                  type="number"
                  min={0}
                  value={rule.guards.minIntervalSecs}
                  onChange={(e) =>
                    setRule((r) => ({
                      ...r,
                      guards: {
                        ...r.guards,
                        minIntervalSecs: Math.max(0, Number(e.target.value)),
                      },
                    }))
                  }
                  style={{ ...inputStyle(), width: "var(--sp-64)" }}
                />
              </Field>
              <Field
                label={t("rotationModal.maxSwaps.label")}
                hint={t("rotationModal.maxSwaps.hint")}
              >
                <input
                  type="number"
                  min={1}
                  value={rule.guards.maxSwapsPerWindow}
                  onChange={(e) =>
                    setRule((r) => ({
                      ...r,
                      guards: {
                        ...r.guards,
                        maxSwapsPerWindow: Math.max(1, Number(e.target.value)),
                      },
                    }))
                  }
                  style={{ ...inputStyle(), width: "var(--sp-64)" }}
                />
              </Field>
              <label
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "var(--sp-8)",
                  fontSize: "var(--fs-sm)",
                }}
              >
                <input
                  type="checkbox"
                  checked={rule.guards.skipWhenCcRunning}
                  onChange={(e) =>
                    setRule((r) => ({
                      ...r,
                      guards: {
                        ...r.guards,
                        skipWhenCcRunning: e.target.checked,
                      },
                    }))
                  }
                />
                {t("rotationModal.skipWhenCc")}
              </label>
            </div>
          </details>

          {/* Dry-run preview */}
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              gap: "var(--sp-8)",
              padding: "var(--sp-12)",
              background: "var(--bg-sunken)",
              borderRadius: "var(--r-3)",
            }}
          >
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
              <strong style={{ fontSize: "var(--fs-sm)" }}>
                {t("rotationModal.testNow")}
              </strong>
              <Button
                variant="ghost"
                glyph={NF.refresh}
                onClick={runDryRun}
                disabled={dryLoading}
              >
                {dryLoading
                  ? t("rotationModal.testing")
                  : t("rotationModal.runDryTest")}
              </Button>
            </div>
            <small style={{ color: "var(--fg-muted)", fontSize: "var(--fs-xs)" }}>
              {t("rotationModal.dryHint")}
            </small>
            {dry && (
              <div
                style={{
                  fontSize: "var(--fs-xs)",
                  padding: "var(--sp-8)",
                  background: dry.wouldFire ? "var(--accent-soft, var(--bg-raised))" : "var(--bg-raised)",
                  borderRadius: "var(--r-2)",
                }}
              >
                <div>
                  <strong>
                    {dry.wouldFire
                      ? t("rotationModal.wouldFire")
                      : t("rotationModal.wouldNotFire")}
                  </strong>
                  {dry.targetEmail ? ` → ${dry.targetEmail}` : ""}
                </div>
                <div style={{ color: "var(--fg-muted)", marginTop: "var(--sp-4)" }}>
                  {dry.reason}
                </div>
              </div>
            )}
          </div>

          {error && (
            <div
              role="alert"
              style={{
                padding: "var(--sp-8) var(--sp-12)",
                background: "var(--bad-weak, var(--bg-raised))",
                color: "var(--danger)",
                fontSize: "var(--fs-sm)",
                borderRadius: "var(--r-2)",
              }}
            >
              {error}
            </div>
          )}
        </div>
      </ModalBody>
      <ModalFooter>
        <Button variant="ghost" onClick={onClose} disabled={saving}>
          {t("shared.cancel")}
        </Button>
        <Button variant="solid" onClick={handleSave} disabled={saving}>
          {saving
            ? t("shared.saving")
            : isEdit
              ? t("rotationModal.save")
              : t("rotationModal.create")}
        </Button>
      </ModalFooter>
    </Modal>
  );
}
