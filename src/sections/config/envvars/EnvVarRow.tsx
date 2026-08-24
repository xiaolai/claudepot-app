// One documented variable: name, safety markers, chooser, help text.
//
// Rows are variable-height by design — help and warning text are the
// point, not decoration — which is why the list measures dynamically
// rather than assuming the Config tree's fixed row height.

import { Trans, useTranslation } from "react-i18next";
import { Glyph } from "../../../components/primitives";
import { NF } from "../../../icons";
import type { Blocked, EnvVarState } from "../../../types/ccEnv";
import { EnvVarControl, type ControlHandlers } from "./EnvVarControl";
import { hasUnnamedHazard, hazardWarning, namedHazards } from "./hazards";

// Every `Blocked` variant needs an entry. `satisfies Record<Blocked, string>`
// makes a new variant a compile error here rather than a row that renders a
// raw key — the row is the only place the reason is explained, so a missing
// one leaves a read-only field with no stated reason, which
// `.claude/rules/design.md` calls out by name.
const BLOCKED_TEXT_KEYS = {
  bootstrap_split_brain: "envvars.blockedBootstrap",
  host_injected: "envvars.blockedHostInjected",
  env_only_not_settings: "envvars.blockedEnvOnly",
} as const satisfies Record<Blocked, string>;

export function EnvVarRow({
  state,
  busy,
  handlers,
}: {
  state: EnvVarState;
  busy: boolean;
  handlers: ControlHandlers;
}) {
  const { t } = useTranslation("config");
  const { spec, settings_value: value, legacy_global, resolved_source } = state;
  const blocked = spec.safety.blocked_reason;
  const hazards = namedHazards(spec.safety.hazards);
  const unestablished = hasUnnamedHazard(spec.safety.hazards);
  const isSet = value.state !== "absent";

  return (
    <div className="envvar-row" data-set={isSet ? "true" : "false"}>
      <div className="envvar-row-head">
        <code className="envvar-name selectable">{spec.name}</code>
        <div className="envvar-tags">
          {isSet ? (
            <span className="envvar-badge" data-tone="set">
              {t("envvars.badgeModified")}
            </span>
          ) : null}
          {spec.safety.secret ? (
            <span className="envvar-badge" data-tone="secret">
              {t("envvars.badgeSecret")}
            </span>
          ) : null}
          {spec.safety.provider_managed ? (
            <span className="envvar-badge" data-tone="muted">
              {t("envvars.badgeProviderManaged")}
            </span>
          ) : null}
          <span className="envvar-badge" data-tone="muted">
            {spec.control}
          </span>
        </div>
        <div className="envvar-control">
          <EnvVarControl
            spec={spec}
            value={value}
            busy={busy}
            handlers={handlers}
          />
        </div>
      </div>

      <p className="envvar-doc">{spec.doc}</p>

      {/* Required disclosure sits inline next to the control, never in a
          tooltip — rules/design.md and rules/icon-buttons.md both treat
          tooltip-as-required-disclosure as an anti-pattern. */}
      {blocked ? (
        <p className="envvar-note" data-tone="warn">
          <Glyph g={NF.lock} /> {t(BLOCKED_TEXT_KEYS[blocked])}
        </p>
      ) : null}
      {hazards.map((h) => (
        <p key={h} className="envvar-note" data-tone="warn">
          <Glyph g={NF.warn} /> {hazardWarning(h)}
        </p>
      ))}
      {/* Absence from Claude Code's allowlist says something is risky
          without saying what. Naming a specific risk here would be the
          same sin as guessing a control type, so this is muted, and
          visibly different from the named hazards above. */}
      {unestablished && hazards.length === 0 && !blocked ? (
        <p className="envvar-note" data-tone="muted">
          {t("envvars.noPreTrust")}
        </p>
      ) : null}
      {spec.safety.provider_managed ? (
        <p className="envvar-note" data-tone="muted">
          {t("envvars.providerManagedNote")}
        </p>
      ) : null}
      {resolved_source === "legacy_global" && legacy_global ? (
        <p className="envvar-note" data-tone="muted">
          <Trans
            ns="config"
            i18nKey="envvars.legacyGlobalWins"
            components={{ code: <code /> }}
          />
        </p>
      ) : null}
      {resolved_source === "settings_override" && legacy_global ? (
        <p className="envvar-note" data-tone="muted">
          <Trans
            ns="config"
            i18nKey="envvars.legacyGlobalShadowed"
            components={{ code: <code /> }}
          />
        </p>
      ) : null}
      {/* "No settings.json override" is deliberately NOT "CC default" —
          the user's shell is a source this pane cannot see. */}
      {!isSet ? (
        <p className="envvar-note" data-tone="muted">
          {t("envvars.noOverride")}
          {spec.default
            ? t("envvars.documentedDefault", { value: spec.default })
            : ""}
          .
        </p>
      ) : null}
    </div>
  );
}
