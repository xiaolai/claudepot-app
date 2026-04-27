import { save } from "@tauri-apps/plugin-dialog";
import { api } from "../../api";
import { formatErrorMessage } from "../../lib/toastError";

export type SessionExportFormat = "md" | "json";

/**
 * Open the native save dialog and write the rendered transcript to
 * the chosen path. Lifted out of the old SessionExportMenu so the
 * detail-header kebab can invoke it without rendering its own
 * dropdown.
 *
 * Cancel is silent (no error). Dialog/permission failures and write
 * failures both flow through `onError` with a clean human line — the
 * raw plugin string is logged to the console instead of toasted.
 *
 * Error strings handed to `onError` are routed through
 * `formatErrorMessage`, which redacts `sk-ant-*` substrings and
 * caps the toast at 240 chars. The Rust side already strips secrets
 * from transcripts before writing, but a user-supplied path or file
 * permission message could in principle echo a secret-shaped
 * substring back through the dialog plugin's error blob — so we
 * defend in depth at the UI seam.
 */
export async function exportSession(
  filePath: string,
  format: SessionExportFormat,
  onError?: (msg: string) => void,
): Promise<void> {
  const defaultName = suggestExportName(filePath, format);
  let target: string | null;
  try {
    target = await save({
      defaultPath: defaultName,
      filters: [
        {
          name: format === "md" ? "Markdown" : "JSON",
          extensions: [format === "md" ? "md" : "json"],
        },
      ],
    });
  } catch (e) {
    // eslint-disable-next-line no-console
    console.error("save() failed:", e);
    onError?.("Couldn't open the save dialog. Please retry.");
    return;
  }
  if (target === null) return;
  try {
    await api.sessionExportToFile(filePath, format, target);
  } catch (e) {
    onError?.(formatErrorMessage("Export failed", e));
  }
}

export function suggestExportName(
  filePath: string,
  format: SessionExportFormat,
): string {
  // Both POSIX and Windows separators — Tauri on Windows sometimes
  // normalises one to the other, so we cannot rely on the host OS.
  const parts = filePath.split(/[\\/]/);
  const base = parts[parts.length - 1] || "session";
  const stem = base.replace(/\.jsonl$/, "");
  return `${stem}.${format === "md" ? "md" : "json"}`;
}
