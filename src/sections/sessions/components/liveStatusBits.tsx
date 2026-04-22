import { useEffect, useState } from "react";
import type { LiveSessionSummary } from "../../../types";

/**
 * Visual atoms for the LiveStatusHeader. Lifted out so the host
 * component stays focused on data lifecycle (subscription, deltas,
 * resync) instead of paint-time styling.
 */

export type ChipTone = "accent" | "neutral" | "warn";

/**
 * Map from CC's canonical live-session status to a chip tone.
 * Errored / stuck overlays are handled separately — they live as
 * booleans on `LiveSessionSummary` and don't displace the base
 * status chip.
 */
export const STATUS_TONE: Record<
  LiveSessionSummary["status"],
  ChipTone
> = {
  busy: "accent",
  waiting: "warn",
  idle: "neutral",
};

/**
 * Pill-shaped status indicator. Border + text track the chosen tone;
 * background stays transparent so the chip overlays cleanly on either
 * surface.
 */
export function Chip({
  tone,
  children,
}: {
  tone: ChipTone;
  children: string;
}) {
  const palette: Record<ChipTone, { fg: string; border: string }> = {
    accent: { fg: "var(--accent)", border: "var(--accent)" },
    warn: { fg: "var(--warn)", border: "var(--warn)" },
    neutral: { fg: "var(--fg-muted)", border: "var(--line)" },
  };
  const p = palette[tone];
  return (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        padding: "2px var(--sp-6)",
        border: `var(--bw-hair) solid ${p.border}`,
        borderRadius: "var(--r-1)",
        color: p.fg,
        fontSize: "var(--fs-xs)",
        fontWeight: 500,
        letterSpacing: "var(--ls-wide)",
      }}
    >
      {children}
    </span>
  );
}

/**
 * Auto-advancing elapsed-time counter for the "idle for N seconds"
 * pill. Bases on the timestamp the backend published, then runs a
 * local rAF tick so the display updates every frame without
 * requiring a backend ping. When `idleMs` changes (a new backend
 * publish), the counter rebases against `performance.now()`.
 */
export function ElapsedCounter({ idleMs }: { idleMs: number }) {
  const [tickMs, setTickMs] = useState(0);
  useEffect(() => {
    setTickMs(0);
    const start = performance.now();
    let rafId: number | null = null;
    const tick = () => {
      setTickMs(performance.now() - start);
      rafId = requestAnimationFrame(tick);
    };
    rafId = requestAnimationFrame(tick);
    return () => {
      if (rafId !== null) cancelAnimationFrame(rafId);
    };
  }, [idleMs]);
  const totalSec = Math.floor((idleMs + tickMs) / 1000);
  const m = Math.floor(totalSec / 60);
  const s = totalSec % 60;
  const text = m > 0 ? `${m}:${String(s).padStart(2, "0")}` : `${s}s`;
  return (
    <span
      style={{
        fontVariantNumeric: "tabular-nums",
        color: "var(--fg-muted)",
      }}
    >
      {text}
    </span>
  );
}
