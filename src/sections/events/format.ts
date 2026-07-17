// Formatting helpers shared by the events/cost surfaces. Hoisted from
// byte-identical copies in CostTab and TopPromptsPanel (audit 2026-07
// F11) so precision/threshold changes happen in exactly one place.

/** Compact token count: 999 → "999", 12_345 → "12.3k",
 *  4_560_000 → "4.56M", 1_200_000_000 → "1.20B". */
export function formatCompact(n: number): string {
  if (n < 1_000) return String(n);
  if (n < 1_000_000) return `${(n / 1_000).toFixed(1)}k`;
  if (n < 1_000_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  return `${(n / 1_000_000_000).toFixed(2)}B`;
}

/** Render the project's basename — the CWD's leaf folder name. CC
 *  project CWDs share long `/Users/<user>/...` prefixes that waste
 *  column width without telling the user anything new; the leaf
 *  folder is what they recognise ("claudepot-app", "vmark"). The
 *  full path is on the row's `title` for hover disclosure.
 *  Windows-aware for `\` separators; trailing separators trimmed. */
export function displayPath(p: string): string {
  if (!p) return p;
  const trimmed = p.replace(/[/\\]+$/, "");
  const segs = trimmed.split(/[/\\]/).filter(Boolean);
  return segs[segs.length - 1] ?? trimmed;
}
