// The two appendix buckets below the documented list.
//
// A cohesive pair in one file, matching the `Table.tsx` /
// `modalParts.tsx` precedent: both answer "what else is in your env
// that this pane cannot offer a control for", and both are read-mostly.

import { Trans, useTranslation } from "react-i18next";
import { Button } from "../../../components/primitives";
import type { EnvOverview } from "../../../types/ccEnv";

/**
 * Keys the user set by hand that are in no list at all.
 *
 * Not in the stated requirements, and a correctness requirement anyway:
 * without it a hand-set key is invisible in a pane that claims to show
 * env config. Values are withheld — an unknown name may be a credential
 * nobody has documented yet.
 */
export function UnrecognizedBucket({
  data,
  busy,
  onClear,
}: {
  data: EnvOverview;
  busy: boolean;
  onClear: (name: string) => void;
}) {
  const { t } = useTranslation("config");
  if (data.unrecognized.length === 0) return null;
  return (
    <section className="envvar-bucket">
      <h3>
        {t("envvars.unrecognizedTitle", { total: data.unrecognized.length })}
      </h3>
      <p className="envvar-bucket-note">
        <Trans
          ns="config"
          i18nKey="envvars.unrecognizedNote"
          values={{ path: data.settings_path }}
          components={{ path: <code className="selectable" /> }}
        />
      </p>
      <ul className="envvar-unrecognized">
        {data.unrecognized.map((u) => (
          <li key={u.name}>
            <code className="selectable">{u.name}</code>
            <span className="envvar-badge" data-tone="muted">
              {t("envvars.badgeSet")} (
              {u.value.state === "withheld" ? u.value.kind : u.value.state})
            </span>
            <Button variant="ghost" disabled={busy} onClick={() => onClear(u.name)}>
              {t("envvars.clear")}
            </Button>
          </li>
        ))}
      </ul>
    </section>
  );
}

/**
 * Names found by scanning a Claude Code binary and documented nowhere.
 *
 * Rendered only on an **exact** version match. These names are
 * non-monotonic — Claude Code can rename or delete one in any release —
 * so a nearest-version match would be a confident-sounding lie rather
 * than an approximation. On a mismatch the section shell stays and says
 * so; no stale name is ever shown.
 *
 * Only the *name list* collapses (see the `<details>` below). The heading,
 * the explanation, and — critically — the mismatch branch are always
 * visible. AGENTS.md requires that a version mismatch render "unavailable
 * for this version"; a disclosure wrapped around that branch would bury the
 * one message the user needs to see, behind a control they have no reason
 * to open.
 */
export function UndocumentedSection({ data }: { data: EnvOverview }) {
  const { t } = useTranslation("config");
  const u = data.undocumented;
  return (
    <section className="envvar-bucket">
      {u.state === "available" && u.names.length === 0 ? (
        <>
          <h3>{t("envvars.undocumentedTitle")}</h3>
          <p className="envvar-bucket-note">
            {t("envvars.undocumentedNoneFound", {
              version: u.snapshot_version,
            })}
          </p>
        </>
      ) : u.state === "available" ? (
        <>
          <h3>
            {t("envvars.undocumentedFoundTitle", {
              version: u.snapshot_version,
              total: u.names.length,
            })}
          </h3>
          <p className="envvar-bucket-note">
            <Trans
              ns="config"
              i18nKey="envvars.undocumentedNote"
              values={{ path: data.settings_path }}
              components={{ path: <code className="selectable" /> }}
            />
          </p>
          {/* The names collapse; the heading and the paragraph above do not.
              This list is the largest thing in the pane by a wide margin —
              293 entries on a current binary — and it is the one part the
              pane explicitly offers no control over. Open by default it
              outranked the 308 variables the user can actually change.

              A native <details> gets keyboard operation, the disclosure
              role, and the platform's own reduced-motion behaviour without
              re-implementing any of it. */}
          <details className="envvar-undocumented-disclosure">
            <summary>
              {t("envvars.showNames", { total: u.names.length })}
            </summary>
            <ul className="envvar-undocumented">
              {u.names.map((n) => (
                <li key={n}>
                  <code className="selectable">{n}</code>
                </li>
              ))}
            </ul>
          </details>
        </>
      ) : (
        // A version mismatch is a state, not an error: the list is
        // "unavailable for this version", never "failed to load".
        <>
          <h3>{t("envvars.undocumentedTitle")}</h3>
          <p className="envvar-bucket-note">
            {t("envvars.undocumentedUnavailable", {
              snapshot: u.snapshot_version,
              installed:
                u.installed_version ?? t("envvars.versionUnresolved"),
            })}
          </p>
        </>
      )}
    </section>
  );
}
