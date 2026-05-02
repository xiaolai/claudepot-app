import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button } from "../../../components/primitives/Button";
import { Modal } from "../../../components/primitives/Modal";

interface Props {
  /** Absolute path to the report file. `null` closes the viewer. */
  path: string | null;
  onClose: () => void;
}

/**
 * Modal markdown viewer for a single report. Reads the file via
 * the existing `templates_read_report` Tauri command (a thin
 * wrapper over std::fs::read_to_string scoped to
 * `~/.claudepot/`). v1 renders the file as plain text in a
 * fixed-width pane; full markdown rendering is a polish item.
 */
export function ReportViewer({ path, onClose }: Props) {
  const [body, setBody] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!path) {
      setBody(null);
      setError(null);
      return;
    }
    let cancelled = false;
    setBody(null);
    setError(null);
    invoke<string>("templates_read_report", { path })
      .then((s) => {
        if (!cancelled) setBody(s);
      })
      .catch((e: unknown) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [path]);

  if (!path) return null;

  return (
    <Modal open={!!path} onClose={onClose} width="lg">
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "var(--sp-12)",
          padding: "var(--sp-16) var(--sp-20)",
        }}
      >
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "center",
          }}
        >
          <h2
            style={{
              margin: 0,
              fontSize: "var(--fs-md)",
              color: "var(--fg)",
              fontFamily: "var(--font-mono)",
            }}
          >
            {basename(path)}
          </h2>
          <Button variant="ghost" onClick={onClose}>
            Close
          </Button>
        </div>

        {error ? (
          <div style={{ color: "var(--danger)", fontSize: "var(--fs-sm)" }}>
            Couldn&rsquo;t read report: {error}
          </div>
        ) : body === null ? (
          <div style={{ color: "var(--fg-faint)", fontSize: "var(--fs-sm)" }}>
            Loading…
          </div>
        ) : (
          <pre
            style={{
              padding: "var(--sp-12)",
              margin: 0,
              border: "var(--bw-hair) solid var(--line)",
              borderRadius: "var(--r-2)",
              background: "var(--bg-sunken)",
              color: "var(--fg)",
              fontSize: "var(--fs-sm)",
              fontFamily: "var(--font-mono)",
              lineHeight: 1.5,
              whiteSpace: "pre-wrap",
              maxHeight: "60vh",
              overflow: "auto",
            }}
          >
            {body}
          </pre>
        )}

        <div style={{ fontSize: "var(--fs-xs)", color: "var(--fg-faint)" }}>
          {path}
        </div>
      </div>
    </Modal>
  );
}

function basename(path: string): string {
  const idx = path.lastIndexOf("/");
  return idx >= 0 ? path.slice(idx + 1) : path;
}
