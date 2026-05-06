/**
 * Source-of-truth registry for the public API surface.
 *
 * This barrel re-exports — exactly once, per (path, method) — the
 * scope, rate-limit bucket, and one-line description for every
 * endpoint under /api/v1, plus the MCP tool list that mirrors them.
 * Three downstream consumers depend on it:
 *
 *   1. Route handlers under app/api/v1/** import the spec via
 *      `endpointSpec("<id>")` and read `spec.scope` / `spec.bucket`
 *      instead of inlining string literals. Changing the manifest
 *      is the only way to change a route's policy.
 *
 *   2. MCP tool registrations under lib/mcp/* import the matching
 *      MCP spec via `mcpToolSpec("<name>")` so REST and MCP can't
 *      drift on scope or bucket.
 *
 *   3. The /api docs page (app/(reader)/api/page.tsx) renders its
 *      tables from the same arrays.
 *
 * A drift test (tests/api-manifest.test.ts) walks the route tree
 * and the MCP tool registrations and asserts a 1:1 match against
 * this manifest — adding a route or tool without a manifest entry
 * (or vice versa) fails CI.
 *
 * The split between types/endpoints/mcp-tools is cosmetic — the
 * data and helpers behave exactly as the prior single-file shape.
 * mcp-tools.ts owns the boot-time invariant assertions so they fire
 * on first import regardless of which barrel surface a consumer
 * touches first.
 */

export type {
  EndpointAuth,
  EndpointId,
  EndpointSpec,
  HttpMethod,
  McpToolName,
  McpToolSpec,
} from "./types";

export { ENDPOINTS, endpointSpec } from "./endpoints";
export { MCP_TOOLS, mcpToolSpec, mcpToolEndpoint } from "./mcp-tools";
