// The (i) affordance.
//
// A disclosure panel, not a native tooltip: `rules/icon-buttons.md`
// treats tooltip-as-required-disclosure as an anti-pattern, and none of
// what follows fits in a tooltip anyway. Every paragraph here is a fact
// a user will otherwise get wrong.

import { Trans, useTranslation } from "react-i18next";
import { ExternalLink } from "../../../components/primitives";
import type { EnvOverview } from "../../../types/ccEnv";

export function EnvVarsDisclosure({ data }: { data: EnvOverview }) {
  const { t } = useTranslation("config");
  return (
    <div className="envvar-disclosure">
      <h4>{t("envvars.disclosure.editsTitle")}</h4>
      <p>
        <Trans
          ns="config"
          i18nKey="envvars.disclosure.editsBody"
          values={{ path: data.settings_path }}
          components={{
            code: <code />,
            path: <code className="selectable" />,
          }}
        />
      </p>

      <h4>{t("envvars.disclosure.precedenceTitle")}</h4>
      <p>
        <Trans
          ns="config"
          i18nKey="envvars.disclosure.precedenceBody"
          components={{ code: <code />, em: <em /> }}
        />
      </p>
      <p>
        <Trans
          ns="config"
          i18nKey="envvars.disclosure.legacyBody"
          components={{ code: <code />, em: <em /> }}
        />
      </p>

      <h4>{t("envvars.disclosure.effectTitle")}</h4>
      {/* Clearing is never live — Claude Code re-applies `env` additively,
          so the old value survives until relaunch. Do not soften this. */}
      <p>
        <Trans
          ns="config"
          i18nKey="envvars.disclosure.effectBody"
          components={{ code: <code />, strong: <strong /> }}
        />
      </p>

      <h4>{t("envvars.disclosure.unsetTitle")}</h4>
      <p>
        <Trans
          ns="config"
          i18nKey="envvars.disclosure.unsetBody"
          components={{ em: <em /> }}
        />
      </p>

      <h4>{t("envvars.disclosure.provenanceTitle")}</h4>
      <ul>
        <li>
          {t("envvars.disclosure.docsFetched", {
            date: data.docs_fetched_at,
          })}{" "}
          <code className="selectable">{data.docs_sha256.slice(0, 16)}…</code>
        </li>
        <li>
          {t("envvars.disclosure.snapshotFrom")}{" "}
          <code>{data.binary_crosscheck_version}</code>
        </li>
        {/* Separate line, not folded into the one above: the safety
            flags have weaker provenance than the binary cross-check and
            no gate that can disable them when stale. Saying so is the
            whole point — see `spec::SafetyProvenance`. */}
        {data.safety_provenance.from_pinned_mirror && (
          <li>
            {t("envvars.disclosure.safetySource", {
              version: data.safety_provenance.mirror_version,
              date: data.safety_provenance.read_at,
            })}
          </li>
        )}
        <li>
          {t("envvars.disclosure.installed")}{" "}
          {data.installed_version ? (
            <code>{data.installed_version}</code>
          ) : (
            t("envvars.disclosure.unresolved")
          )}
          {data.installed_path ? (
            <>
              {t("envvars.disclosure.atPath")}
              <code className="selectable">{data.installed_path}</code>
            </>
          ) : null}
        </li>
      </ul>
      <p>{t("envvars.disclosure.lifetimesNote")}</p>

      <p>
        <ExternalLink href="https://code.claude.com/docs/en/env-vars">
          {t("envvars.disclosure.officialReference")}
        </ExternalLink>
      </p>
    </div>
  );
}
