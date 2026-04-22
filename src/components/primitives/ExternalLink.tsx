import { openUrl } from "@tauri-apps/plugin-opener";
import type { CSSProperties, ReactNode } from "react";

type Props = {
  href: string;
  children: ReactNode;
  "aria-label"?: string;
};

export function ExternalLink({ href, children, ...rest }: Props) {
  return (
    <button
      type="button"
      onClick={() => {
        void openUrl(href).catch(() => {});
      }}
      style={style}
      aria-label={rest["aria-label"]}
    >
      {children}
    </button>
  );
}

const style: CSSProperties = {
  background: "transparent",
  border: "none",
  padding: 0,
  color: "var(--accent)",
  textDecoration: "underline",
  cursor: "pointer",
  font: "inherit",
};
