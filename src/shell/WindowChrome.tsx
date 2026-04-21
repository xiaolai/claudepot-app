import type { MouseEvent } from "react";
import { Glyph } from "../components/primitives/Glyph";
import { IconButton } from "../components/primitives/IconButton";
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
 * `data-tauri-drag-region` on the outer strip makes every pixel a
 * window-drag handle; interactive children stop mousedown from
 * propagating so they behave as buttons, not drag seeds. The
 * breadcrumb inherits drag from the parent without needing its own
 * attribute, so the text stays selectable.
 * Left padding clears the OS traffic lights via `--chrome-inset-left`.
 */
export function WindowChrome({
  cwd,
  theme,
  onToggleTheme,
  onCmdK,
}: WindowChromeProps) {
  const stopDrag = (e: MouseEvent) => e.stopPropagation();
  return (
    <div
      data-tauri-drag-region
      style={{
        height: "var(--chrome-height)",
        display: "flex",
        alignItems: "center",
        // The hairline border-bottom eats 1px from the content box
        // under `box-sizing: border-box`; mirroring it with
        // padding-top restores a symmetric content box so
        // `alignItems: center` lands on the true chrome centerline
        // (y=19 in a 38px strip) — which is where the OS centers
        // the traffic lights via `trafficLightPosition.y`.
        padding:
          "var(--bw-hair) var(--sp-12) 0 var(--chrome-inset-left)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        background: "var(--bg)",
        flexShrink: 0,
        gap: "var(--sp-14)",
        userSelect: "none",
      }}
    >
      {/* breadcrumb / cwd — drag inherits from the outer strip */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-6)",
          fontSize: "var(--fs-sm)",
          lineHeight: "var(--lh-flat)",
          color: "var(--fg-muted)",
        }}
      >
        {/* NF glyph ink sits ~6–8% above the geometric centre of its
            em-box; verticalAlign on Glyph is a no-op in flex, so we
            apply the baseline nudge here via transform to re-center
            the ink against the traffic-light row and the text. */}
        <Glyph
          g={NF.home}
          color="var(--fg-faint)"
          style={{ transform: "translateY(var(--glyph-optical-nudge))" }}
        />
        <span style={{ color: "var(--fg)", fontWeight: 500 }}>{cwd}</span>
      </div>

      <div style={{ flex: 1 }} />

      {/* command palette hint — no fill, hairline side rules only
          (left + right), ⌘K reads as flat text. Border-radius
          preserves the rounded cap on each vertical stroke. */}
      <button
        type="button"
        onClick={onCmdK}
        onMouseDown={stopDrag}
        aria-label="Open command palette"
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-10)",
          height: "var(--sp-26)",
          padding: "0 var(--sp-8) 0 var(--sp-10)",
          fontSize: "var(--fs-xs)",
          color: "var(--fg-faint)",
          background: "transparent",
          border: "none",
          borderLeft: "var(--bw-hair) solid var(--line)",
          borderRight: "var(--bw-hair) solid var(--line)",
          borderRadius: "var(--r-2)",
          minWidth: "var(--banner-min-width)",
          cursor: "pointer",
        }}
      >
        <Glyph g={NF.search} />
        <span style={{ flex: 1, textAlign: "left" }}>Jump to anything</span>
        <span
          style={{
            fontFamily: "var(--font)",
            fontSize: "var(--fs-2xs)",
            fontWeight: 500,
            color: "var(--fg-muted)",
          }}
        >
          ⌘K
        </span>
      </button>

      <IconButton
        glyph={theme === "dark" ? NF.sun : NF.moon}
        onClick={onToggleTheme}
        onMouseDown={stopDrag}
        title="Toggle theme"
        aria-label={
          theme === "dark" ? "Switch to light mode" : "Switch to dark mode"
        }
        // Glyph scales off the button's own font-size. --fs-xl (22px)
        // was correct in the Nerd Font era — NF glyphs rendered at
        // ~65% of font-size, so that read as ~14px inside the 28px
        // square. Lucide SVGs fill the full box, so --fs-md (14px)
        // restores the prior half-of-button proportion.
        style={{ fontSize: "var(--fs-md)" }}
      />
    </div>
  );
}
