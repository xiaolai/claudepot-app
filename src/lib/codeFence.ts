/**
 * Is this `<code>` className tagged with `language-<name>`?
 *
 * `className.includes("language-mermaid")` — which three renderers used
 * independently — is a substring test on a space-delimited class list,
 * so `language-mermaidish` matched and an ordinary fence was handed to
 * the diagram renderer. Split on whitespace and compare whole tokens.
 *
 * There is a second copy in `panel/src/app/mdConfig.js`, because the
 * panel is a separate Vite app that cannot import from `src/`. Both are
 * pinned by the same cases in their respective tests; change one,
 * change the other.
 */
export function hasLanguage(className: string | undefined, language: string): boolean {
  if (!className) return false;
  return className.split(/\s+/).includes(`language-${language}`);
}
