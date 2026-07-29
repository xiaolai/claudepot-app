// The (i) affordance.
//
// A disclosure panel, not a native tooltip: `rules/icon-buttons.md`
// treats tooltip-as-required-disclosure as an anti-pattern, and none of
// what follows fits in a tooltip anyway. Every paragraph here is a fact
// a user will otherwise get wrong.

import { ExternalLink } from "../../../components/primitives";
import type { EnvOverview } from "../../../types/ccEnv";

export function EnvVarsDisclosure({ data }: { data: EnvOverview }) {
  return (
    <div className="envvar-disclosure">
      <h4>What this edits</h4>
      <p>
        The <code>env</code> block of{" "}
        <code className="selectable">{data.settings_path}</code>. That path is
        resolved, not assumed — <code>CLAUDE_CONFIG_DIR</code> moves it.
        Project-scoped settings are deliberately not editable here: Claude Code
        applies only its safe-variable allowlist from a project file, because
        those live inside the repository and could be committed by someone
        else.
      </p>

      <h4>Precedence</h4>
      <p>
        In a normal CLI session Claude Code applies settings <code>env</code>{" "}
        after the launch environment, so it usually overrides your shell
        export. Managed, host-spawned and <code>claude ssh</code> contexts
        filter or supersede values. Whether an environment variable beats the
        matching settings <em>field</em> is per-variable, not a rule.
      </p>
      <p>
        <code>~/.claude.json</code> carries its own <code>env</code> block,
        which Claude Code applies <em>first</em> — so this file wins where both
        set a key, and that one still takes effect where this file is silent.
        Rows it affects say so.
      </p>

      <h4>When it takes effect</h4>
      <p>
        Claude Code re-applies settings <code>env</code> to a running session{" "}
        <strong>additively</strong>. Setting or changing a value is usually
        live. <strong>Clearing one never is</strong> — nothing is deleted from
        a running session's environment, so the old value survives until you
        relaunch Claude Code.
      </p>

      <h4>Unset is not zero</h4>
      <p>
        Claude Code's default for nearly every variable is the key being
        absent, not a value. <em>Restore default</em> therefore removes the
        key; it never writes the documented default, which would pin today's
        number into your settings and override whatever Claude Code changes it
        to later. An explicit empty string is a third state again, distinct
        from both.
      </p>

      <h4>Where these facts come from</h4>
      <ul>
        <li>
          Official documentation fetched {data.docs_fetched_at}, sha256{" "}
          <code className="selectable">{data.docs_sha256.slice(0, 16)}…</code>
        </li>
        <li>
          Undocumented-name snapshot taken from Claude Code{" "}
          <code>{data.binary_crosscheck_version}</code>
        </li>
        <li>
          Your installed Claude Code:{" "}
          {data.installed_version ? (
            <code>{data.installed_version}</code>
          ) : (
            "could not be resolved"
          )}
          {data.installed_path ? (
            <>
              {" at "}
              <code className="selectable">{data.installed_path}</code>
            </>
          ) : null}
        </li>
      </ul>
      <p>
        These are two separate facts with different lifetimes — the live
        documentation already lists variables absent from any given build — so
        neither date describes the other.
      </p>

      <p>
        <ExternalLink href="https://code.claude.com/docs/en/env-vars">
          Official reference
        </ExternalLink>
      </p>
    </div>
  );
}
