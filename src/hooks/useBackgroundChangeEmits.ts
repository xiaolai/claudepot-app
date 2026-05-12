// Phase 2 fix for audit issue #1: memory:changed and config-tree-patch
// events used to be consumed only by mounted panes (MemoryPane,
// useConfigTree's subscriber inside ConfigSection). If the user was on
// any other section while CC rewrote ~/.claude/CLAUDE.md or an
// automation patched ~/.claude/settings.json, the event vanished —
// no toast, no banner, no bell entry.
//
// This hook wires both events at app-state level (parallel to
// useRotationEvents) and emits them through the notification facade
// as P3 ambient categories. Per route(), P3 is log-only — the
// bell-icon popover surfaces them without spraying toasts or OS
// banners for routine writes.

import { useEffect } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { useEmit } from "../providers/AppStateProvider";

/** Mirrors the Rust event payload at `memory_watch.rs::emit_memory_changed`. */
interface MemoryChangedPayload {
  abs_path: string;
  role: string;
  change_type: "created" | "modified" | "deleted";
  project_slug: string | null;
  kind_hint: string;
}

/** Subset of the config-tree-patch payload `useConfigTree` already
 *  consumes — only the fields we surface in the bell entry. */
interface ConfigTreePatchPayload {
  added?: { parent_scope_id: string; file?: { abs_path?: string } }[];
  updated?: { abs_path?: string }[];
  removed?: string[];
  full_snapshot?: unknown;
}

/**
 * Wire background-change events into the notification log. Mounted
 * once at App.tsx next to `useRotationEvents`.
 *
 * Why no toast/banner: P3 ambient categories route to log-only by
 * design. Routine `CLAUDE.md` writes from CC or `settings.json`
 * edits from an automation should be queryable in the bell, not
 * interrupt the user. The user opens the bell popover when they
 * want to ask "what did Claudepot just record for me?" — the
 * design.md "render-if-nonzero" rule applies to spray, not to
 * persistence.
 */
export function useBackgroundChangeEmits(): void {
  const emit = useEmit();

  useEffect(() => {
    let active = true;
    const unlisteners: UnlistenFn[] = [];

    const wire = <T>(channel: string, handler: (p: T) => void) => {
      void listen<T>(channel, (ev) => {
        if (!active || !ev.payload) return;
        handler(ev.payload);
      })
        .then((fn) => {
          if (!active) fn();
          else unlisteners.push(fn);
        })
        .catch(() => {
          /* non-tauri env */
        });
    };

    wire<MemoryChangedPayload>("memory:changed", (p) => {
      // Filter ChangeType::Deleted's "no path" emits — CC sometimes
      // emits transient deletions during atomic writes that
      // immediately re-create. Keep the bell entry for true
      // deletes but skip continuous-noise edits where the file no
      // longer exists. We can't tell apart without reading the FS
      // again, so we keep all events but use `change_type` to
      // render a clearer title.
      const verb =
        p.change_type === "created"
          ? "Created"
          : p.change_type === "deleted"
            ? "Deleted"
            : "Edited";
      const scope = p.project_slug ? `project (${p.project_slug})` : "global";
      void emit({
        category: "memoryChanged",
        title: `${verb} CLAUDE.md — ${scope}`,
        body: p.abs_path,
        dedupeKey: `memory:${p.abs_path}:${p.change_type}`,
      });
    });

    wire<ConfigTreePatchPayload>("config-tree-patch", (p) => {
      // Skip full_snapshot patches — those are the initial hydrate,
      // not a user-visible change. We only log incremental diffs.
      if (p.full_snapshot) return;
      const added = p.added?.length ?? 0;
      const updated = p.updated?.length ?? 0;
      const removed = p.removed?.length ?? 0;
      if (added + updated + removed === 0) return;
      const parts: string[] = [];
      if (added) parts.push(`+${added}`);
      if (updated) parts.push(`~${updated}`);
      if (removed) parts.push(`-${removed}`);
      // First path we can surface — purely cosmetic for the bell row.
      const firstPath =
        p.added?.[0]?.file?.abs_path ?? p.updated?.[0]?.abs_path ?? "";
      void emit({
        category: "configTreePatched",
        title: `Config file changed (${parts.join(" ")})`,
        body: firstPath,
        // Dedup on patch shape — repeated saves of the same file
        // collapse into one bell row inside the rate window.
        dedupeKey: `config:${firstPath || "any"}`,
      });
    });

    return () => {
      active = false;
      unlisteners.forEach((fn) => fn());
    };
  }, [emit]);
}
