/**
 * Random pale tint generator for card-grid surfaces.
 *
 * Returns a CSS color suitable for the `--card-tint` custom property
 * read by `.proto-project-card-tinted` (see prototype.css). Low
 * saturation + high lightness keeps it as a barely-tinted off-white
 * that never competes with the text. The hue is the only thing that
 * varies per card.
 *
 * Called from Server Components, so each page render reshuffles the
 * palette — no client-side hydration mismatch concerns because the
 * value is committed on the server before HTML ships.
 */
export function randomCardTint(): string {
  const hue = Math.floor(Math.random() * 360);
  return `hsl(${hue} 22% 97%)`;
}
