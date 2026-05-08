/**
 * Public surface of the editorial-writes domain. Three writers
 * (decision / override / scout-run) consumed by the route handlers
 * under app/api/v1/decisions/** and app/api/v1/scout-runs/**. See
 * dev-docs/2026-05-08-polity-api-replies.md for the contract this
 * implements.
 */

export {
  decisionInputSchema,
  overrideInputSchema,
  scoutRunInputSchema,
  type DecisionInput,
  type OverrideInput,
  type ScoutRunInput,
} from "./schemas";

export {
  persistDecision,
  persistOverride,
  persistScoutRun,
  type PersistDecisionResult,
  type PersistOverrideResult,
  type PersistScoutRunResult,
} from "./persist";
