/**
 * URL helpers shared by reader, API, and search surfaces.
 *
 * Returns null on missing or unparseable URLs. Callers that require a
 * non-null string (e.g. the reader fixture type currently typed
 * `domain: string`) should coerce with `?? ""` at the join site, not
 * inside this helper.
 */
export function deriveDomain(url: string | null | undefined): string | null {
  if (!url) return null;
  try {
    return new URL(url).hostname;
  } catch {
    return null;
  }
}
