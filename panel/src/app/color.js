// Colours mermaid can actually read.
//
// Every token in this design system is `oklch()`, and mermaid runs each
// theme colour through **khroma** — `adjust`, `darken`, `isDark` — to
// derive borders, contrast and hover states. khroma predates CSS Color 4
// and throws `Unsupported color format` on an oklch string.
//
// The symptom is not an error anywhere near the colour: theme
// construction throws before layout, so the whole diagram fails and the
// reader is told it could not be drawn. Measured — a sequence and a
// state diagram rendered fine while a flowchart did not, because the
// flowchart theme derives more colours and reached a throwing path
// first. That near-miss is why this is converted rather than hoped for.
//
// ## Why the transform is written out rather than delegated
//
// The browser could do it — set the colour on a probe element and read
// back `getComputedStyle`, or round-trip it through a canvas fillStyle.
// Both depend on how a specific engine chooses to serialise a wide-gamut
// colour, which is exactly the kind of thing that differs between Safari
// and Chrome and cannot be verified from a development machine. The
// transform is a published one and is ~25 lines; doing it here makes the
// output the same everywhere and testable without a browser.
//
// ## The other copy
//
// `src/sections/config/mermaidColor.ts` is the same function for the
// desktop app, which has the same palette and the same mermaid. They are
// not pinned by shared vectors the way `session::title` is, and the
// reason is a real difference: that one encodes *our* judgement about
// what a title should look like, where this one implements CSS Color 4.
// Both test against the sRGB primaries, whose oklch coordinates are
// published and cannot drift.

/** `oklch(…)`, with or without an alpha slash, in either notation. */
const OKLCH = /^oklch\(\s*([^\s)]+)\s+([^\s)]+)\s+([^\s/)]+)\s*(?:\/\s*([^\s)]+)\s*)?\)$/i;

/** A CSS number that may be a percentage. `none` is zero, per the spec. */
function num(raw, percentBasis) {
  if (raw === undefined || raw === null) return 0;
  const s = String(raw).trim();
  if (s === 'none' || s === '') return 0;
  if (s.endsWith('%')) return (parseFloat(s) / 100) * percentBasis;
  const v = parseFloat(s);
  return Number.isFinite(v) ? v : 0;
}

/** Gamma-encode one linear-light channel and quantise to 0–255. */
function encode(linear) {
  const c =
    linear <= 0.0031308 ? 12.92 * linear : 1.055 * Math.pow(Math.max(linear, 0), 1 / 2.4) - 0.055;
  return Math.round(Math.min(1, Math.max(0, c)) * 255);
}

/**
 * Oklch → sRGB. Björn Ottosson's Oklab, as specified by CSS Color 4.
 *
 * Out-of-gamut colours are clipped per channel rather than gamut-mapped.
 * A theme colour that leaves sRGB is already outside what mermaid's SVG
 * can express, and clipping keeps the hue recognisable.
 */
export function oklchToRgb(L, C, hueDeg) {
  const h = (hueDeg * Math.PI) / 180;
  const a = C * Math.cos(h);
  const b = C * Math.sin(h);

  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;

  return [
    encode(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
    encode(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
    encode(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s),
  ];
}

/**
 * A colour mermaid can parse.
 *
 * Anything that is not `oklch()` is returned untouched — hex, `rgb()`,
 * `hsl()`, a named colour and `transparent` all already work, and so
 * does a non-colour like a font stack, which makes this safe to map over
 * every theme variable rather than a curated subset.
 */
export function toMermaidColor(value) {
  if (typeof value !== 'string') return value;
  const raw = value.trim();
  const m = OKLCH.exec(raw);
  if (!m) return value;

  const [, lRaw, cRaw, hRaw, aRaw] = m;
  const [r, g, b] = oklchToRgb(num(lRaw, 1), num(cRaw, 0.4), num(hRaw, 1));

  if (aRaw !== undefined) {
    // khroma reads `rgba()`. Alpha matters here: `--hair` is
    // `oklch(24% 0.01 60/0.09)`, and dropping its transparency would
    // paint every hairline as near-black.
    const alpha = Math.min(1, Math.max(0, num(aRaw, 1)));
    return `rgba(${r}, ${g}, ${b}, ${Number(alpha.toFixed(4))})`;
  }
  return `#${[r, g, b].map((c) => c.toString(16).padStart(2, '0')).join('')}`;
}
