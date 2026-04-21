import { useCallback, useEffect, useRef, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import { NF } from "../../icons";

/**
 * Export-format dropdown next to the Reveal/Move buttons in the
 * session header. Fires Tauri's native save dialog, then tells
 * `session_export_to_file` to render and write the body — the Rust
 * side handles `sk-ant-*` redaction and 0600 permissions on Unix.
 *
 * Cancelling the save dialog is silent (no error toast). If the render
 * itself or the write fails, the error is surfaced via `onError`.
 */
export function SessionExportMenu({
  filePath,
  onError,
}: {
  filePath: string;
  onError?: (msg: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open]);

  const doExport = useCallback(
    async (format: "md" | "json") => {
      setBusy(true);
      setOpen(false);
      try {
        const defaultName = suggestName(filePath, format);
        let target: string | null = null;
        try {
          target = await save({
            defaultPath: defaultName,
            filters: [
              {
                name: format === "md" ? "Markdown" : "JSON",
                extensions: [format === "md" ? "md" : "json"],
              },
            ],
          });
        } catch {
          target = null;
        }
        if (!target) return; // user cancelled — silent
        try {
          await api.sessionExportToFile(filePath, format, target);
        } catch (e) {
          onError?.(`Export failed: ${String(e)}`);
        }
      } finally {
        setBusy(false);
      }
    },
    [filePath, onError],
  );

  return (
    <div ref={rootRef} style={{ position: "relative" }}>
      <Button
        variant="ghost"
        glyph={NF.download}
        glyphColor="var(--fg-muted)"
        onClick={() => setOpen((v) => !v)}
        disabled={busy}
      >
        Export
      </Button>
      {open && (
        <div
          role="menu"
          style={{
            position: "absolute",
            top: "calc(100% + var(--sp-4))",
            left: 0,
            zIndex: 10,
            minWidth: 160,
            background: "var(--bg-raised)",
            border: "var(--bw-hair) solid var(--line)",
            borderRadius: "var(--r-2)",
            boxShadow: "var(--shadow-popover, 0 4px 12px rgba(0,0,0,0.15))",
            padding: "var(--sp-4) 0",
            fontSize: "var(--fs-sm)",
          }}
        >
          <button
            type="button"
            role="menuitem"
            onClick={() => doExport("md")}
            style={menuItemStyle}
          >
            Markdown
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => doExport("json")}
            style={menuItemStyle}
          >
            JSON
          </button>
        </div>
      )}
    </div>
  );
}

const menuItemStyle: React.CSSProperties = {
  display: "block",
  width: "100%",
  textAlign: "left",
  padding: "var(--sp-6) var(--sp-12)",
  background: "transparent",
  border: 0,
  color: "var(--fg)",
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: "inherit",
};

function suggestName(filePath: string, format: "md" | "json"): string {
  const base = filePath.split("/").pop() ?? "session";
  const stem = base.replace(/\.jsonl$/, "");
  const ext = format === "md" ? "md" : "json";
  return `${stem}.${ext}`;
}
