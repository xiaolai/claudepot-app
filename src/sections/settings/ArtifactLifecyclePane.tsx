// Settings → Cleanup → Artifacts. Orchestration only — the disabled
// list, trash list, and presentational primitives live in sibling
// files so each shard stays under the loc-guardian limit.

import { useCallback, useEffect, useState } from "react";
import { api } from "../../api";
import type { DisabledRecordDto, TrashEntryDto } from "../../types";
import { ArtifactTrashList, PURGE_AFTER_DAYS } from "./ArtifactTrashList";
import { DisabledArtifactList } from "./DisabledArtifactList";
import { Empty, Section } from "./LifecyclePresentational";

export function ArtifactLifecyclePane({
  pushToast,
  projectRoot,
}: {
  pushToast: (kind: "info" | "error", text: string) => void;
  /** Active project anchor, when one is set. Forwarded to
   * `artifact_list_disabled` so project-scoped disabled artifacts
   * surface alongside user-scoped ones. */
  projectRoot: string | null;
}) {
  const [disabled, setDisabled] = useState<DisabledRecordDto[] | null>(null);
  const [trash, setTrash] = useState<TrashEntryDto[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [refreshTick, setRefreshTick] = useState(0);

  const refresh = useCallback(() => setRefreshTick((n) => n + 1), []);

  useEffect(() => {
    let cancelled = false;
    setLoadError(null);
    Promise.all([
      api.artifactListDisabled(projectRoot),
      api.artifactListTrash(),
    ])
      .then(([d, t]) => {
        if (cancelled) return;
        setDisabled(d);
        setTrash(t);
      })
      .catch((err) => {
        if (cancelled) return;
        setLoadError(err instanceof Error ? err.message : String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [projectRoot, refreshTick]);

  // One-time auto-purge of old Healthy entries on mount. Quiet —
  // toast only when something is actually purged.
  useEffect(() => {
    let cancelled = false;
    api
      .artifactPurgeTrash(PURGE_AFTER_DAYS)
      .then((n) => {
        if (cancelled || n === 0) return;
        pushToast(
          "info",
          `Auto-purged ${n} trash entr${n === 1 ? "y" : "ies"} older than ${PURGE_AFTER_DAYS} days`,
        );
        refresh();
      })
      .catch(() => {
        // Silent: cleanup is best-effort.
      });
    return () => {
      cancelled = true;
    };
    // refresh and pushToast are stable from the parent.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (loadError) {
    return (
      <Section title="Artifacts">
        <Empty danger>Couldn't load artifact lifecycle: {loadError}</Empty>
      </Section>
    );
  }

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-24)",
      }}
    >
      <DisabledArtifactList
        rows={disabled}
        projectRoot={projectRoot}
        pushToast={pushToast}
        onChanged={refresh}
      />
      <ArtifactTrashList
        rows={trash}
        pushToast={pushToast}
        onChanged={refresh}
      />
    </div>
  );
}
