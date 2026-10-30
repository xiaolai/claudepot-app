// A labelled form field for the Remote and Relay panes: a small-caps
// caption above its control, the whole wrapped in a <label> so clicking
// the caption focuses the input. `Input` draws its own chrome, so this
// is only the caption. One implementation; the two panes had identical
// copies.
import type { ReactNode } from "react";

export function SettingsField({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="remote-field">
      <span className="mono-cap" style={{ color: "var(--fg-faint)" }}>
        {label}
      </span>
      {children}
    </label>
  );
}
