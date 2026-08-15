import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { renderError } from "../../lib/i18n-error";

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
  const { t } = useTranslation("projects");
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
        if (!cancelled) setError(renderError(e));
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
          color: "var(--fg-muted)",
          fontSize: "var(--fs-sm)",
        }}
      >
        {t("templates.noSample", { error })}
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
        {t("templates.loadingSample")}
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
