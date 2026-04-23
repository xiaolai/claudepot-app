import { useCallback, useEffect, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import type {
  ConfigKind,
  ConfigTreeDto,
  EditorCandidateDto,
  EditorDefaultsDto,
} from "../types";
import { ScreenHeader } from "../shell/ScreenHeader";
import { PreviewHeader } from "../components/primitives/PreviewHeader";
import { Button } from "../components/primitives/Button";
import { NF } from "../icons";

interface ConfigSectionProps {
  subRoute: string | null;
  onSubRouteChange: (subRoute: string | null) => void;
}

/**
 * Config section — read-only browser over CC's filesystem artifacts.
 *
 * P0 ships the section shell: empty state + "Open with…" split-button
 * primitive + editor detection. Scope-first tree, preview renderers,
 * watcher, secret masking, and the effective-settings view land in
 * later phases (see `dev-docs/config-section-plan.md` §15).
 *
 * `subRoute` is wired now so later phases can select nodes via
 * `node:<id>` without reshuffling the call sites.
 */
export function ConfigSection(_props: ConfigSectionProps) {
  const [tree, setTree] = useState<ConfigTreeDto | null>(null);
  const [editors, setEditors] = useState<EditorCandidateDto[] | null>(null);
  const [defaults, setDefaults] = useState<EditorDefaultsDto | null>(null);
  const [toast, setToast] = useState<string | null>(null);

  useEffect(() => {
    void api.configScan(null).then(setTree).catch(() => setTree(null));
    void api
      .configListEditors(false)
      .then(setEditors)
      .catch(() => setEditors([]));
    void api
      .configGetEditorDefaults()
      .then(setDefaults)
      .catch(() =>
        setDefaults({ by_kind: {}, fallback: "system" }),
      );
  }, []);

  useEffect(() => {
    if (!toast) return;
    const h = window.setTimeout(() => setToast(null), 4000);
    return () => window.clearTimeout(h);
  }, [toast]);

  const refreshEditors = useCallback(() => {
    setEditors(null);
    void api
      .configListEditors(true)
      .then(setEditors)
      .catch(() => setEditors([]));
  }, []);

  const pickOther = useCallback(async () => {
    try {
      const picked = await openDialog({
        multiple: false,
        title: "Choose editor binary",
      });
      if (typeof picked !== "string") return;
      // "Other…" on a P0 stub section doesn't have a target file yet
      // — surface a toast and save the picked path as the fallback
      // default so future opens use it.
      setToast(`Set fallback editor to: ${picked}`);
      // P0 doesn't yet persist user-picked binaries as new candidates;
      // that lands with the file-backed preview in P1. For now, tell
      // the user to use system default.
    } catch {
      setToast("Could not open file picker");
    }
  }, []);

  const openPath = useCallback(
    async (path: string, editorId: string | null, kind: ConfigKind | null) => {
      try {
        await api.configOpenInEditorPath(path, editorId, kind);
      } catch (err) {
        setToast(String(err));
      }
    },
    [],
  );

  const setDefault = useCallback(
    async (kind: ConfigKind | null, editorId: string) => {
      try {
        await api.configSetEditorDefault(kind, editorId);
        const next = await api.configGetEditorDefaults();
        setDefaults(next);
        setToast(
          kind
            ? `Default editor for ${kind} set to ${editorId}`
            : `Fallback editor set to ${editorId}`,
        );
      } catch (err) {
        setToast(String(err));
      }
    },
    [],
  );

  const claudeDir = tree?.cwd ? `${tree.cwd}/.claude` : null;

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        minHeight: 0,
      }}
    >
      <ScreenHeader
        title="Config"
        subtitle="Read-only browser over Claude Code's filesystem artifacts."
        actions={
          <Button
            variant="ghost"
            glyph={NF.refresh}
            onClick={refreshEditors}
            title="Re-probe installed editors"
          >
            Refresh editors
          </Button>
        }
      />

      <div
        style={{
          flex: 1,
          display: "flex",
          flexDirection: "column",
          padding: "var(--sp-24) var(--sp-32)",
          gap: "var(--sp-20)",
          overflow: "auto",
        }}
      >
        <div
          style={{
            border: "var(--bw-hair) solid var(--line)",
            borderRadius: "var(--r-2)",
            background: "var(--bg-sunken)",
          }}
        >
          <PreviewHeader
            title="Config home"
            subtitle="The directory Claude Code reads on every launch."
            path={claudeDir}
            kind={null}
            editors={editors}
            defaults={defaults}
            onOpen={(editorId) => {
              if (!claudeDir) return;
              void openPath(claudeDir, editorId, null);
            }}
            onPickOther={pickOther}
            onSetDefault={setDefault}
            onRefreshEditors={refreshEditors}
          />
          <EmptyState />
        </div>
      </div>

      {toast && (
        <div
          role="status"
          aria-live="polite"
          style={{
            position: "fixed",
            bottom: "var(--sp-24)",
            right: "var(--sp-24)",
            padding: "var(--sp-8) var(--sp-12)",
            background: "var(--bg-elev)",
            border: "var(--bw-hair) solid var(--line-strong)",
            borderRadius: "var(--r-2)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg)",
            boxShadow: "var(--shadow-md)",
            maxWidth: 360,
          }}
        >
          {toast}
        </div>
      )}
    </div>
  );
}

function EmptyState() {
  return (
    <div
      style={{
        padding: "var(--sp-28) var(--sp-20)",
        textAlign: "center",
        color: "var(--fg-faint)",
        fontSize: "var(--fs-sm)",
      }}
    >
      <div style={{ marginBottom: "var(--sp-6)" }}>
        Scope-first tree + previews land in the next phase.
      </div>
      <div style={{ fontSize: "var(--fs-xs)" }}>
        For now, "Open in…" is wired so you can jump straight to the files
        in your preferred editor.
      </div>
    </div>
  );
}
