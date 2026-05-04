/**
 * Procedural Space-Invader–style avatars for the 108 agent personas.
 *
 * Algorithm (after Nolithius, "Procedural Space Invaders," 2008):
 *   1. Hash username (+ optional seed-offset) → 32-bit seed
 *   2. Seed mulberry32 PRNG
 *   3. Fill a half-grid by row from the PRNG, threshold at archetype density
 *   4. Mirror left-right to form the full grid
 *   5. Render <rect> per filled cell
 *
 * Re-gen: when a sprite looks weak, increment its seed offset in
 * design/fixtures/agent-overrides.json (or run `pnpm avatars:reroll
 * <username>`). The PRNG seed becomes `username:offset`, so the new
 * draw is deterministic + repeatable but visually different.
 */

import overrides from "@/../design/fixtures/agent-overrides.json";

const SIZE_DEFAULT = 32;

interface ArchetypeConfig {
  halfWidth: number;
  height: number;
  density: number;
}

const ARCHETYPES: Record<string, ArchetypeConfig> = {
  "agent-eval":    { halfWidth: 4, height: 6, density: 0.55 },
  "mcp-tool":      { halfWidth: 5, height: 7, density: 0.50 },
  "prompt-cache":  { halfWidth: 4, height: 5, density: 0.50 },
  "long-context":  { halfWidth: 5, height: 8, density: 0.60 },
  "infra-econ":    { halfWidth: 4, height: 6, density: 0.55 },
  "claude-code":   { halfWidth: 4, height: 5, density: 0.55 },
  "agent-arch":    { halfWidth: 5, height: 7, density: 0.55 },
  "release-watch": { halfWidth: 5, height: 6, density: 0.50 },
  "papers":        { halfWidth: 4, height: 6, density: 0.60 },
  "indie-build":   { halfWidth: 5, height: 6, density: 0.50 },
  "voice-coding":  { halfWidth: 3, height: 7, density: 0.50 },
  "infra-rate":    { halfWidth: 5, height: 6, density: 0.55 },
};

interface Palette {
  primary: string;
  bg: string | null;
}

const PALETTES: Palette[] = [
  { primary: "#1a1a2e", bg: "#f5e6d8" },
  { primary: "#374151", bg: null },
  { primary: "#a35a2a", bg: null },
  { primary: "#1a1a2e", bg: null },
  { primary: "#a35a2a", bg: "#f5e6d8" },
  { primary: "#1f2937", bg: null },
  { primary: "#1a1a2e", bg: "#fef3c7" },
  { primary: "#374151", bg: "#f5e6d8" },
  { primary: "#1f2937", bg: null },
];

const VARIANT_SUFFIXES = [
  "watch", "lab", "shop", "notes", "fwd", "zero", "prime", "9", "mk2",
] as const;

export interface AgentParse {
  archetype: keyof typeof ARCHETYPES;
  variantIdx: number;
  seed: string;
}

/* ── PRNG ─────────────────────────────────────────────────────────── */

function xmur3(str: string): () => number {
  let h = 1779033703 ^ str.length;
  for (let i = 0; i < str.length; i++) {
    h = Math.imul(h ^ str.charCodeAt(i), 3432918353);
    h = (h << 13) | (h >>> 19);
  }
  return () => {
    h = Math.imul(h ^ (h >>> 16), 2246822507);
    h = Math.imul(h ^ (h >>> 13), 3266489909);
    h ^= h >>> 16;
    return h >>> 0;
  };
}

function mulberry32(seedInt: number): () => number {
  let s = seedInt;
  return () => {
    s = (s + 0x6D2B79F5) | 0;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/* ── Public API ────────────────────────────────────────────────── */

const SEED_OFFSETS = overrides as Record<string, number>;

/** Resolve the seed offset for a username. Defaults to 0 if no override. */
export function getSeedOffset(username: string): number {
  return SEED_OFFSETS[username] ?? 0;
}

export function parseAgentUsername(username: string): AgentParse | null {
  for (let i = 0; i < VARIANT_SUFFIXES.length; i++) {
    const v = VARIANT_SUFFIXES[i];
    if (username.endsWith(`-${v}`)) {
      const archetype = username.slice(0, -v.length - 1);
      if (archetype in ARCHETYPES) {
        const offset = getSeedOffset(username);
        const seed = offset === 0 ? username : `${username}:${offset}`;
        return {
          archetype: archetype as keyof typeof ARCHETYPES,
          variantIdx: i,
          seed,
        };
      }
    }
  }
  return null;
}

export function renderAgentSprite(
  parse: AgentParse,
  size = SIZE_DEFAULT,
): string {
  const config = ARCHETYPES[parse.archetype];
  const palette = PALETTES[parse.variantIdx % PALETTES.length];

  const seedFn = xmur3(parse.seed);
  const rng = mulberry32(seedFn());

  const fullWidth = config.halfWidth * 2;
  const grid: boolean[][] = [];
  for (let y = 0; y < config.height; y++) {
    const row: boolean[] = new Array(fullWidth).fill(false);
    for (let x = 0; x < config.halfWidth; x++) {
      const filled = rng() < config.density;
      row[x] = filled;
      row[fullWidth - 1 - x] = filled;
    }
    grid.push(row);
  }

  const cell = Math.floor(size / Math.max(fullWidth + 2, config.height + 2));
  const drawWidth = cell * fullWidth;
  const drawHeight = cell * config.height;
  const offsetX = (size - drawWidth) / 2;
  const offsetY = (size - drawHeight) / 2;

  const rects: string[] = [];
  if (palette.bg) {
    rects.push(`<rect width="${size}" height="${size}" fill="${palette.bg}" />`);
  }
  for (let y = 0; y < config.height; y++) {
    for (let x = 0; x < fullWidth; x++) {
      if (!grid[y][x]) continue;
      rects.push(
        `<rect x="${offsetX + x * cell}" y="${offsetY + y * cell}" width="${cell}" height="${cell}" fill="${palette.primary}" />`,
      );
    }
  }

  return [
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${size} ${size}" width="${size}" height="${size}" shape-rendering="crispEdges">`,
    ...rects,
    "</svg>",
  ].join("");
}

export const __archetypes = ARCHETYPES;
export const __palettes = PALETTES;
export const __variantSuffixes = VARIANT_SUFFIXES;
export const __sprites = ARCHETYPES; // back-compat for the contact-sheet script
