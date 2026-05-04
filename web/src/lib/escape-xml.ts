/**
 * XML/HTML attribute-safe escape. Used by RSS feed routes and the
 * weekly digest email body. Single source of truth for these five
 * substitutions — duplicated copies in 4 files were collapsed here
 * (audit finding 1.3).
 */
export function escapeXml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}
