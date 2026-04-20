import { Glyph } from "../components/primitives/Glyph";
import { IconButton } from "../components/primitives/IconButton";
import { Kbd } from "../components/primitives/Kbd";
import { NF } from "../icons";

interface WindowChromeProps {
  /** Breadcrumb tail — "accounts", "projects", etc. */
  cwd: string;
  /** "light" or "dark" — controls sun/moon icon. */
  theme: "light" | "dark";
  onToggleTheme: () => void;
  /** Opens the command palette. */
  onCmdK: () => void;
}

/**
 * Top chrome (height `--chrome-height`, 38px): breadcrumb on the
 * left, ⌘K palette hint center-right, theme toggle far right.
 * `data-tauri-drag-region` makes the whole strip a window-drag
 * handle; buttons opt out via stopPropagation on their own events.
 * Left padding clears the OS traffic lights via `--chrome-inset-left`.
 */
export function WindowChrome({
  cwd,
  theme,
  onToggleTheme,
  onCmdK,
}: WindowChromeProps) {
  return (
    <div
      data-tauri-drag-region
      style={{
        height: "var(--chrome-height)",
        display: "flex",
        alignItems: "center",
        padding:
          "0 var(--sp-12) 0 var(--chrome-inset-left)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        background: "var(--bg)",
        flexShrink: 0,
        gap: "var(--sp-14)",
        userSelect: "none",
      }}
    >
      {/* breadcrumb / cwd */}
      <div
        data-tauri-drag-region
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-6)",
          fontSize: "var(--fs-sm)",
          color: "var(--fg-muted)",
          pointerEvents: "none",
        }}
      >
        <Glyph g={NF.home} color="var(--fg-faint)" />
        <span>~/.claude</span>
        <Glyph
          g={NF.chevronR}
          color="var(--fg-ghost)"
          style={{ fontSize: "var(--fs-2xs)" }}
        />
        <span style={{ color: "var(--fg)", fontWeight: 500 }}>{cwd}</span>
      </div>

      <div data-tauri-drag-region style={{ flex: 1 }} />

      {/* command palette hint */}
      <button
        type="button"
        onClick={onCmdK}
        aria-label="Open command palette"
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-10)",
          height: "var(--sp-26)",
          padding: "0 var(--sp-8) 0 var(--sp-10)",
          fontSize: "var(--fs-xs)",
          color: "var(--fg-faint)",
          background: "var(--bg-sunken)",
          border: "var(--bw-hair) solid var(--line)",
          borderRadius: "var(--r-2)",
          minWidth: "var(--banner-min-width)",
          cursor: "pointer",
        }}
      >
        <Glyph g={NF.search} />
        <span style={{ flex: 1, textAlign: "left" }}>Jump to anything</span>
        <Kbd>⌘K</Kbd>
      </button>

      <IconButton
        glyph={theme === "dark" ? NF.sun : NF.moon}
        onClick={onToggleTheme}
        title="Toggle theme"
        aria-label={
          theme === "dark" ? "Switch to light mode" : "Switch to dark mode"
        }
      />
    </div>
  );
}
