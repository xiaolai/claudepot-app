import type { CSSProperties } from "react";

/** Hairline 1px `--line` divider. Thin by design. */
export function Divider({ style }: { style?: CSSProperties }) {
  return (
    <div
      style={{
        height: "var(--bw-hair)",
        background: "var(--line)",
        flexShrink: 0,
        ...style,
      }}
    />
  );
}
