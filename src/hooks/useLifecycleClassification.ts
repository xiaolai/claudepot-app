import { useEffect, useState } from "react";
import { api } from "../api";
import type { ClassifyPathDto } from "../types";

/**
 * Classify the given absolute path via `artifact_classify_path`,
 * returning the resulting Trackable + already_disabled flag, or
 * the typed RefuseReason. Used by the FilePreview header to gate
 * Disable / Trash actions; recomputes when `absPath` or
 * `projectRoot` changes.
 *
 * Returns `null` while in flight so the renderer can suppress
 * action buttons until the classification arrives.
 */
export function useLifecycleClassification(
  absPath: string | null,
  projectRoot: string | null,
): ClassifyPathDto | null {
  const [result, setResult] = useState<ClassifyPathDto | null>(null);

  useEffect(() => {
    if (!absPath) {
      setResult(null);
      return;
    }
    let cancelled = false;
    setResult(null);
    api
      .artifactClassifyPath(absPath, projectRoot)
      .then((r) => {
        if (cancelled) return;
        setResult(r);
      })
      .catch(() => {
        // On bridge failure, surface as a generic refusal so the UI
        // hides the action buttons rather than offering ones that'd
        // immediately fail.
        if (cancelled) return;
        setResult({
          trackable: null,
          refused: "classification unavailable",
          already_disabled: false,
        });
      });
    return () => {
      cancelled = true;
    };
  }, [absPath, projectRoot]);

  return result;
}
