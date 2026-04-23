import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type {
  ConfigFileNodeDto,
  ConfigScopeNodeDto,
  ConfigTreeDto,
} from "../types";

/**
 * Backend `config-tree-patch` payload (mirrors
 * `src-tauri/src/config_watch.rs::ConfigTreePatchEvent`).
 */
export interface ConfigTreePatchEvent {
  generation: number;
  added: { parent_scope_id: string; file: ConfigFileNodeDto }[];
  updated: ConfigFileNodeDto[];
  removed: string[];
  reordered: { parent_scope_id: string; child_ids: string[] }[];
  full_snapshot: {
    scopes: ConfigScopeNodeDto[];
    cwd: string;
    project_root: string;
    config_home_dir: string;
    memory_slug: string;
    memory_slug_lossy: boolean;
  } | null;
  dirty_during_emit: boolean;
}

interface UseConfigTreeResult {
  tree: ConfigTreeDto | null;
  /** True while the watcher last emitted mid-settling. UI can show "updating…". */
  dirty: boolean;
  /** Increment this to inject an out-of-band snapshot (e.g. after config_scan). */
  setTree: (tree: ConfigTreeDto | null) => void;
  /**
   * Increments every time a non-snapshot patch arrives without a baseline
   * tree present. Consumers watch this with a useEffect and trigger a
   * fresh configScan to recover — otherwise an early scan failure would
   * leave every subsequent watcher patch silently dropped.
   */
  orphanPatchSignal: number;
}

/**
 * Subscribe to `config-tree-patch` events and apply them to an
 * in-memory `ConfigTreeDto`. Each patch is applied atomically; the
 * caller seeds the tree once via `setTree(initialSnapshot)` (typically
 * the `config_scan` result) and this hook takes over from there.
 *
 * Policy (plan §11.5):
 * - `full_snapshot` replaces the tree outright.
 * - Otherwise apply removed → updated → added → reordered in that
 *   order so every id in `added`/`updated` names a currently-living
 *   file.
 */
export function useConfigTree(initial: ConfigTreeDto | null): UseConfigTreeResult {
  const [tree, setTree] = useState<ConfigTreeDto | null>(initial);
  const [dirty, setDirty] = useState(false);
  // True if we received a non-snapshot patch before the tree was
  // seeded. The consumer watches this signal and kicks off a recovery
  // scan — see ConfigSection.tsx. Stored as a counter so repeated
  // orphan patches don't retrigger a second recovery before the first
  // scan's setTree lands.
  const [orphanPatchSignal, setOrphanPatchSignal] = useState(0);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void listen<ConfigTreePatchEvent>("config-tree-patch", (ev) => {
      if (cancelled) return;
      const payload = ev.payload;
      setDirty(payload.dirty_during_emit);
      if (payload.full_snapshot) {
        setTree({
          scopes: payload.full_snapshot.scopes,
          cwd: payload.full_snapshot.cwd,
          project_root: payload.full_snapshot.project_root,
          config_home_dir: payload.full_snapshot.config_home_dir,
          memory_slug: payload.full_snapshot.memory_slug,
          memory_slug_lossy: payload.full_snapshot.memory_slug_lossy,
        });
        return;
      }
      // Functional setState: each patch sees the latest tree even when
      // multiple patches arrive before a render lands. Orphan patches
      // (no baseline yet) bump `orphanPatchSignal` so the consumer can
      // trigger a recovery scan.
      setTree((prev) => {
        if (!prev) {
          setOrphanPatchSignal((n) => n + 1);
          return prev;
        }
        return applyPatch(prev, payload);
      });
    }).then((u) => {
      if (cancelled) {
        u();
      } else {
        unlisten = u;
      }
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  return { tree, dirty, setTree, orphanPatchSignal };
}

export function applyPatch(
  prev: ConfigTreeDto,
  patch: ConfigTreePatchEvent,
): ConfigTreeDto {
  if (
    patch.removed.length === 0 &&
    patch.updated.length === 0 &&
    patch.added.length === 0 &&
    patch.reordered.length === 0
  ) {
    return prev;
  }

  const removedSet = new Set(patch.removed);
  const updatedById = new Map<string, ConfigFileNodeDto>();
  for (const f of patch.updated) updatedById.set(f.id, f);

  const nextScopes: ConfigScopeNodeDto[] = prev.scopes.map((s) => {
    let files = s.files.filter((f) => !removedSet.has(f.id));
    if (updatedById.size > 0) {
      files = files.map((f) => updatedById.get(f.id) ?? f);
    }
    return { ...s, files };
  });

  // Apply added — to the scope with matching parent_scope_id.
  if (patch.added.length > 0) {
    const addsByScope = new Map<string, ConfigFileNodeDto[]>();
    for (const a of patch.added) {
      const bucket = addsByScope.get(a.parent_scope_id) ?? [];
      bucket.push(a.file);
      addsByScope.set(a.parent_scope_id, bucket);
    }
    for (const scope of nextScopes) {
      const adds = addsByScope.get(scope.id);
      if (adds) {
        scope.files = [...scope.files, ...adds];
      }
    }
  }

  // Apply reordered — canonical child order supplied by core.
  if (patch.reordered.length > 0) {
    const reorderByScope = new Map<string, string[]>();
    for (const r of patch.reordered) {
      reorderByScope.set(r.parent_scope_id, r.child_ids);
    }
    for (const scope of nextScopes) {
      const order = reorderByScope.get(scope.id);
      if (order) {
        const byId = new Map(scope.files.map((f) => [f.id, f]));
        scope.files = order
          .map((id) => byId.get(id))
          .filter((f): f is ConfigFileNodeDto => f !== undefined);
      }
    }
  }

  // Refresh recursive_count so the sidebar count reflects reality.
  for (const scope of nextScopes) {
    scope.recursive_count = scope.files.length;
  }

  return { ...prev, scopes: nextScopes };
}
