/**
 * Protocols a link in model output may use.
 *
 * `react-markdown`'s default `urlTransform` strips `javascript:` and a
 * few known-dangerous schemes, but it is an allowlist of *dangerous*
 * things rather than of safe ones: relative URLs pass, and so do
 * `irc:`, `ircs:` and `xmpp:`. Both transcript renderers relied on that
 * default while their comments claimed a narrower set.
 *
 * On the desktop that mattered most: `ExternalLink` hands the href to
 * the OS opener, so an `xmpp:` URL in a transcript launches whatever
 * app registered the scheme. The input here is model output quoting
 * arbitrary files, which is exactly the input that must not choose a
 * protocol handler.
 *
 * A relative URL is refused too — there is no meaningful base for one
 * inside a transcript, and in the Tauri webview it would navigate the
 * application itself.
 *
 * **Matched lexically, with no `new URL()`.** Parsing needs a base, and
 * a base is a URL literal — which is a remote origin sitting in the
 * panel bundle, where `remote::assets`'s "no third-party requests" test
 * correctly refuses to have one. A scheme is a prefix; test the prefix.
 * Anything that does not match is refused, so an unparseable or
 * exotically-encoded value fails closed rather than open.
 *
 * Twin of `isSafeHref` in `panel/src/app/mdConfig.js`; change one,
 * change the other.
 */
const ALLOWED_SCHEME = /^(?:https?|mailto):/i;

export function isSafeHref(href: string | undefined): boolean {
  if (!href) return false;
  return ALLOWED_SCHEME.test(href.trim());
}
