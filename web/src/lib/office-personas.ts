/**
 * Persona descriptions for the /office/ public pages.
 *
 * Mirrors editorial/rubric.yml v0.2.3 persona_overlays. Hardcoded here
 * (not loaded from rubric.yml at runtime) for simplicity at v0; if the
 * mapping drifts, regenerate from the YAML or load it server-side.
 *
 * Tested in tests/office-personas.test.ts.
 */

export interface OfficePersona {
  id: string;
  display: string;
  description: string;
  multipliers: Record<string, number>;
}

export const OFFICE_PERSONAS: Record<string, OfficePersona> = {
  ada: {
    id: "ada",
    display: "ada",
    description:
      "Evidence-first skeptic. Asks 'where's the eval?'. Strongest on engineer / infra-shipper content.",
    multipliers: {
      evidence_quality: 1.5,
      mechanism_specificity: 1.2,
      counter_current: 0.8,
    },
  },
  historian: {
    id: "historian",
    display: "historian",
    description:
      "Pattern-matcher. Surfaces déjà-vu and what already broke. Cross-cutting.",
    multipliers: {
      counter_current: 2.0,
      author_credibility: 1.3,
      recency_bonus: 0.5,
    },
  },
  scout: {
    id: "scout",
    display: "scout",
    description:
      "Cross-domain pattern translator. 'This screenwriter's prompt trick works for marketers too.' Finds non-obvious sources outside the engineer mainstream — strongest on knowledge-worker / operator content.",
    multipliers: {
      domain_legibility: 1.5,
      diversity_bonus: 1.5,
      practitioner_fit: 1.2,
      author_credibility: 0.8,
    },
  },
  base: {
    id: "base",
    display: "base",
    description:
      "No persona overlay applied. The rubric's base weights as written.",
    multipliers: {},
  },
};
