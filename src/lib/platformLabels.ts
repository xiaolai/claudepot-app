/**
 * UI labels that change with the host OS. Keep the table small and
 * the choices documented here so reviewers don't have to guess why a
 * label reads one way on macOS and another on Windows.
 *
 * Inputs accept the strings emitted by `AppStatus.platform` from the
 * Rust side: "macos" | "linux" | "windows". Anything else falls back
 * to the generic label.
 */

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
      return "Finder";
    case "windows":
      return "File Explorer";
    case "linux":
      return "Files";
    default:
      return "File Manager";
  }
}
