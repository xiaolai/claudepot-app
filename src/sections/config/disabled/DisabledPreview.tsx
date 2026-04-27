// Right column of the Disabled scope view: targeted preview of the
// selected disabled artifact via `artifact_disabled_preview`. The
// regular Config preview path can't reach `.disabled/` entries (by
// design — the active discovery deny-list excludes them).

import { useEffect, useState } from "react";
import { api } from "../../../api";
import type { DisabledRecordDto, LifecycleKind } from "../../../types";

export function DisabledPreview({
  row,
  projectRoot,
}: {
  row: DisabledRecordDto | null;
  projectRoot: string | null;
}) {
  const [body, setBody] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!row) {
      setBody(null);
      setError(null);
      return;
    }
    let cancelled = false;
    setBody(null);
    setError(null);
    api
      .artifactDisabledPreview(
        row.scope_root,
        row.kind as LifecycleKind,
        row.name,
        projectRoot,
      )
      .then((b) => {
        if (cancelled) return;
        setBody(b);
      })
      .catch((err) => {
        if (cancelled) return;
        setError(err instanceof Error ? err.message : String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [row, projectRoot]);

  return (
    <div
      style={{
        overflow: "auto",
        minHeight: 0,
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        background: "var(--bg)",
      }}
    >
      {!row ? (
        <PreviewEmpty>Select a disabled artifact to preview.</PreviewEmpty>
      ) : error ? (
        <PreviewEmpty danger>Couldn't load preview: {error}</PreviewEmpty>
      ) : body === null ? (
        <PreviewEmpty>Loading…</PreviewEmpty>
      ) : (
        <>
          <header
            style={{
              padding: "var(--sp-8) var(--sp-12)",
              borderBottom: "var(--bw-hair) solid var(--line)",
              background: "var(--bg-sunken)",
              fontSize: "var(--fs-2xs)",
              color: "var(--fg-faint)",
              letterSpacing: "var(--ls-wide)",
              textTransform: "uppercase",
            }}
            title={row.current_path}
          >
            {row.kind} · {row.name}
          </header>
          <pre
            style={{
              margin: 0,
              padding: "var(--sp-12)",
              fontFamily: "var(--font-mono)",
              fontSize: "var(--fs-xs)",
              color: "var(--fg)",
              whiteSpace: "pre-wrap",
              overflowWrap: "anywhere",
            }}
          >
            {body}
          </pre>
        </>
      )}
    </div>
  );
}

function PreviewEmpty({
  children,
  danger,
}: {
  children: React.ReactNode;
  danger?: boolean;
}) {
  return (
    <div
      style={{
        padding: "var(--sp-20)",
        textAlign: "center",
        color: danger ? "var(--danger)" : "var(--fg-faint)",
        fontSize: "var(--fs-sm)",
      }}
    >
      {children}
    </div>
  );
}
