/**
 * UI labels that change with the host OS. Keep the table small and
 * the choices documented here so reviewers don't have to guess why a
 * label reads one way on macOS and another on Windows.
 *
 * Inputs accept the strings emitted by `AppStatus.platform` from the
 * Rust side: "macos" | "linux" | "windows". Anything else falls back
 * to the generic label.
 *
 * The names below are the ENGLISH choices; each localized catalog
 * carries the OS vendor's own name for that language (macOS ships
 * Finder as 访达 in Simplified Chinese, for example), which is what a
 * user actually sees in their menu bar.
 */
import { i18n } from "./i18n";

/**
 * Native file manager name for the host OS.
 *
 * Choices:
 *   - "Finder"        — macOS system app name (Apple HIG).
 *   - "File Explorer" — official Windows 10/11 name; menu strings in
 *                        Windows ship as "Show in File Explorer".
 *   - "Files"         — matches the GNOME default app; reads cleanly
 *                        on KDE/Xfce too where the actual binary
 *                        differs (Dolphin, Thunar, Nemo, …).
 *   - "File Manager"  — truly generic fallback for the brief window
 *                        before AppStatus has loaded (`platform` is
 *                        `undefined`) or for an unrecognized OS. Picking
 *                        any specific name here would mislabel the host
 *                        for users on the other platforms.
 */
export function fileManagerName(platform: string | undefined): string {
  switch (platform) {
    case "macos":
      return i18n.t("platform.fileManager.macos");
    case "windows":
      return i18n.t("platform.fileManager.windows");
    case "linux":
      return i18n.t("platform.fileManager.linux");
    default:
      return i18n.t("platform.fileManager.generic");
  }
}
