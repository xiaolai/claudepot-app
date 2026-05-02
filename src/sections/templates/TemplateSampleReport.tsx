import { useEffect, useState } from "react";
import { api } from "../../api";

interface Props {
  templateId: string;
}

/**
 * Renders the bundled sample report for a template. Plain-text
 * markdown for v1 — no syntax highlighting, no live rendering.
 * The point is to show the user what the actual output looks
 * like before they install; readability beats prettiness.
 */
export function TemplateSampleReport({ templateId }: Props) {
  const [md, setMd] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setMd(null);
    setError(null);
    api
      .templatesSampleReport(templateId)
      .then((s) => {
        if (!cancelled) setMd(s);
      })
      .catch((e: unknown) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [templateId]);

  if (error) {
    return (
      <div
        style={{
          padding: "var(--sp-12)",
          border: "var(--bw-hair) solid var(--line)",
          borderRadius: "var(--r-2)",
          color: "var(--fg-2)",
          fontSize: "var(--fs-sm)",
        }}
      >
        No sample available: {error}
      </div>
    );
  }
  if (md === null) {
    return (
      <div
        style={{
          padding: "var(--sp-12)",
          color: "var(--fg-faint)",
          fontSize: "var(--fs-sm)",
        }}
      >
        Loading sample…
      </div>
    );
  }

  return (
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
        lineHeight: "var(--lh-code)",
        whiteSpace: "pre-wrap",
        overflowX: "auto",
        maxHeight: "32vh",
        overflowY: "auto",
      }}
    >
      {md}
    </pre>
  );
}
