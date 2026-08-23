// Colours mermaid can actually read.
//
// Every token in tokens.css is `oklch()`, and mermaid runs each theme
// colour through **khroma** — `adjust`, `darken`, `isDark` — to derive
// borders, contrast and hover states. khroma predates CSS Color 4 and
// throws `Unsupported color format` on an oklch string.
//
// The symptom is not an error anywhere near the colour: theme
// construction throws before layout, so the whole diagram fails. Latent
// here since `MermaidBlock` was written, because it takes a diagram
// whose theme derives enough colours to reach a throwing path — a
// flowchart does, a sequence diagram does not, and the failure was found
// on the remote panel rather than in the Config preview.
//
// ## Why the transform is written out rather than delegated
//
// The browser could do it — a probe element and `getComputedStyle`, or a
// canvas fillStyle round-trip. Both depend on how a specific engine
// chooses to serialise a wide-gamut colour, which differs between
// engines and cannot be verified from a development machine. The
// transform is published and is ~25 lines; doing it here makes the
// output identical everywhere and testable without a browser.
//
// ## The other copy
//
// `panel/src/app/color.js` is the same function for the remote panel,
// which is its own install and cannot import from here. They are not
// pinned by shared vectors the way `session::title` is, and the reason
// is a real difference: that one encodes *our* judgement about what a
// title should look like, where this one implements CSS Color 4. Both
// test against the sRGB primaries, whose oklch coordinates are published
// and cannot drift.

/** `oklch(…)`, with or without an alpha slash, in either notation. */
const OKLCH = /^oklch\(\s*([^\s)]+)\s+([^\s)]+)\s+([^\s/)]+)\s*(?:\/\s*([^\s)]+)\s*)?\)$/i;

/** A CSS number that may be a percentage. `none` is zero, per the spec. */
function num(raw: string | undefined, percentBasis: number): number {
  if (raw === undefined || raw === null) return 0;
  const s = String(raw).trim();
  if (s === "none" || s === "") return 0;
  if (s.endsWith("%")) return (parseFloat(s) / 100) * percentBasis;
  const v = parseFloat(s);
  return Number.isFinite(v) ? v : 0;
}

/** Gamma-encode one linear-light channel and quantise to 0–255. */
function encode(linear: number): number {
  const c =
    linear <= 0.0031308 ? 12.92 * linear : 1.055 * Math.pow(Math.max(linear, 0), 1 / 2.4) - 0.055;
  return Math.round(Math.min(1, Math.max(0, c)) * 255);
}

/**
 * Oklch → sRGB. Björn Ottosson"s Oklab, as specified by CSS Color 4.
 *
 * Out-of-gamut colours are clipped per channel rather than gamut-mapped.
 * A theme colour that leaves sRGB is already outside what mermaid"s SVG
 * can express, and clipping keeps the hue recognisable.
 */
export function oklchToRgb(L: number, C: number, hueDeg: number): [number, number, number] {
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
export function toMermaidColor<T>(value: T): T | string {
  if (typeof value !== "string") return value;
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
  return `#${[r, g, b].map((c) => c.toString(16).padStart(2, "0")).join("")}`;
}
