import { api } from "../../api";
import type { ContextMenuItem } from "../../components/ContextMenu";
import type { SessionRow } from "../../types";

/**
 * Build the right-click menu for a session row. Pure function — every
 * dependency (toast, selection, move-session setter) is passed in,
 * which keeps the menu testable and the parent's render path tight.
 *
 * Menu shape:
 *   Open in Finder
 *   Open in detail
 *   ──────────────
 *   Move to project…    (guarded on `project_from_transcript`)
 *   ──────────────
 *   Copy session id
 *   Copy project path
 */
export function buildSessionContextMenuItems(args: {
  session: SessionRow;
  setToast: (msg: string) => void;
  setSelectedPath: (path: string) => void;
  setMoveSession: (row: SessionRow) => void;
  copyToClipboard: (text: string, label: string) => void;
}): ContextMenuItem[] {
  const { session: s, setToast, setSelectedPath, setMoveSession, copyToClipboard } = args;
  return [
    {
      label: "Open in Finder",
      onClick: () => {
        api.revealInFinder(s.file_path).catch((e) => {
          setToast(`Couldn't reveal: ${e}`);
        });
      },
    },
    {
      label: "Open in detail",
      onClick: () => setSelectedPath(s.file_path),
    },
    { label: "", separator: true, onClick: () => {} },
    {
      // Identity tuple for the move is `(session_id, from_cwd)`
      // — sufficient because the backend resolves to
      // `~/.claude/projects/{sanitize(from_cwd)}/{session_id}.jsonl`,
      // which is a single on-disk file. The only failure mode would
      // be a from_cwd derived via the lossy `unsanitize` fallback
      // (`.claude/rules/paths.md`), which is exactly what the
      // `project_from_transcript` guard rules out here.
      label: "Move to project…",
      onClick: () => {
        if (!s.project_from_transcript) {
          setToast(
            "Can't move: this session has no cwd recorded in the transcript.",
          );
          return;
        }
        setMoveSession(s);
      },
    },
    { label: "", separator: true, onClick: () => {} },
    {
      label: "Copy session id",
      onClick: () => copyToClipboard(s.session_id, "session id"),
    },
    {
      label: "Copy project path",
      onClick: () => copyToClipboard(s.project_path, "project path"),
    },
  ];
}
