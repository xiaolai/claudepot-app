// The two appendix buckets below the documented list.
//
// A cohesive pair in one file, matching the `Table.tsx` /
// `modalParts.tsx` precedent: both answer "what else is in your env
// that this pane cannot offer a control for", and both are read-mostly.

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
  if (data.unrecognized.length === 0) return null;
  return (
    <section className="envvar-bucket">
      <h3>Set here, documented nowhere ({data.unrecognized.length})</h3>
      <p className="envvar-bucket-note">
        These keys are in your settings file but match no documented
        variable. Their values are not shown: an unrecognized name could be a
        credential. To see or change one, edit{" "}
        <code className="selectable">{data.settings_path}</code> by hand.
      </p>
      <ul className="envvar-unrecognized">
        {data.unrecognized.map((u) => (
          <li key={u.name}>
            <code className="selectable">{u.name}</code>
            <span className="envvar-badge" data-tone="muted">
              set (
              {u.value.state === "withheld" ? u.value.kind : u.value.state})
            </span>
            <Button variant="ghost" disabled={busy} onClick={() => onClear(u.name)}>
              Clear
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
  const u = data.undocumented;
  return (
    <section className="envvar-bucket">
      {u.state === "available" && u.names.length === 0 ? (
        <>
          <h3>Documented nowhere</h3>
          <p className="envvar-bucket-note">
            Nothing undocumented was found in Claude Code {u.snapshot_version}.
          </p>
        </>
      ) : u.state === "available" ? (
        <>
          <h3>
            Documented nowhere — found in Claude Code {u.snapshot_version} (
            {u.names.length})
          </h3>
          <p className="envvar-bucket-note">
            These names were found by scanning your installed Claude Code
            binary. Anthropic does not document them. They can change or
            disappear in any release, their accepted values are unknown, and
            setting one carries no safety guarantee. Claudepot deliberately
            offers no control for them — guessing a type or a value would be
            worse than offering nothing. To set one anyway, edit{" "}
            <code className="selectable">{data.settings_path}</code> by hand.
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
            <summary>Show {u.names.length} names</summary>
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
        <>
          <h3>Documented nowhere</h3>
          <p className="envvar-bucket-note">
            Unavailable — this list was built against Claude Code{" "}
            {u.snapshot_version} and you're running{" "}
            {u.installed_version ?? "a version Claudepot could not resolve"}.
            These names can be renamed or removed in any release, so a list from
            a different version would be a guess rather than a finding.
          </p>
        </>
      )}
    </section>
  );
}
