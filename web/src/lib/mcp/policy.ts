/**
 * Manifest-driven policy enforcement for MCP tools.
 *
 * The REST surface has lib/api/policy.ts; this file is the matching
 * pair for MCP. Both read scope/bucket from lib/api/manifest.ts so a
 * tool and its sister REST endpoint can never disagree on what
 * scope they require or which bucket they charge.
 *
 * Auth comes from `extra.authInfo` (populated by withMcpAuth and
 * verifyClaudepotToken in lib/mcp/auth.ts). The handler shape is
 * MCP's textResult — we return one on rejection so the call site
 * can short-circuit without further wrapping.
 */

import { checkAndIncrement, type LimitCategory } from "@/lib/api/rate-limit";
import {
  mcpToolEndpoint,
  type McpToolName,
} from "@/lib/api/manifest";
import type { ClaudepotAuthExtra } from "./auth";

export type McpReadyCtx = {
  userId: string;
  tokenId: string;
  tokenPrefix: string;
  username: string;
  role: string;
  isStaff: boolean;
  isAgent: boolean;
  botKind: string | null;
};

type TextResult = {
  isError: boolean;
  content: Array<{ type: "text"; text: string }>;
};

function textResult(text: string, isError = false): TextResult {
  return { isError, content: [{ type: "text" as const, text }] };
}

function getAuthExtra(extra: {
  authInfo?: { extra?: Record<string, unknown> };
}): ClaudepotAuthExtra | null {
  const ai = extra.authInfo?.extra;
  if (
    !ai ||
    typeof ai.userId !== "string" ||
    typeof ai.username !== "string" ||
    typeof ai.role !== "string" ||
    typeof ai.isAgent !== "boolean" ||
    typeof ai.tokenId !== "string" ||
    typeof ai.tokenPrefix !== "string"
  ) {
    return null;
  }
  // botKind is text|null on the schema; normalize unknown shapes to
  // null so the mcp tool gates can branch safely.
  const botKind = typeof ai.botKind === "string" ? ai.botKind : null;
  return { ...(ai as unknown as ClaudepotAuthExtra), botKind };
}

const RATE_LIMIT_NOUN: Record<LimitCategory, string> = {
  reads: "read",
  submissions: "submission",
  comments: "comment",
  votes: "vote",
  saves: "save",
  bots: "bot-report",
};

/**
 * Verify auth + scope per the manifest. Returns a ready context on
 * success or a textResult to return immediately. Calling this on a
 * tool whose mirrored endpoint is "public" is a programmer error
 * (MCP requires auth) and throws.
 */
export async function checkAuthForTool(
  toolName: McpToolName,
  extra: {
    authInfo?: { extra?: Record<string, unknown>; scopes?: string[] };
  },
): Promise<{ ok: true; ctx: McpReadyCtx } | { ok: false; result: TextResult }> {
  const SPEC = mcpToolEndpoint(toolName);
  if (SPEC.auth === "public") {
    throw new Error(
      `checkAuthForTool: tool "${toolName}" mirrors a public endpoint. ` +
        `MCP tools always require auth — adjust the manifest or the tool.`,
    );
  }
  const auth = getAuthExtra(extra);
  if (!auth) {
    return { ok: false, result: textResult("Unauthorized.", true) };
  }
  if (SPEC.auth !== "any") {
    // SPEC.auth narrowed to Scope here.
    const scopes = extra.authInfo?.scopes ?? [];
    if (!scopes.includes(SPEC.auth)) {
      return {
        ok: false,
        result: textResult(
          `Forbidden: this token is missing the ${SPEC.auth} scope.`,
          true,
        ),
      };
    }
  }
  return {
    ok: true,
    ctx: {
      userId: auth.userId,
      tokenId: auth.tokenId,
      tokenPrefix: auth.tokenPrefix,
      username: auth.username,
      role: auth.role,
      // Match lib/api/policy.ts:isStaffAuth — both `staff` (humans)
      // and `system` (Ada and other agent accounts) are staff-
      // equivalent. Without `system` here, the same agent account
      // sees different staff-only access through MCP than through
      // the REST surface — same asymmetry the Codex audit caught
      // earlier on isStaffAuth.
      isStaff: auth.role === "staff" || auth.role === "system",
      isAgent: auth.isAgent,
      botKind: auth.botKind,
    },
  };
}

/**
 * Charge the manifest's bucket. No-op when the spec's bucket is null
 * (e.g. `me`, `get_quota`). Returns a textResult on overflow.
 */
export async function chargeForTool(
  toolName: McpToolName,
  tokenId: string,
): Promise<{ ok: true } | { ok: false; result: TextResult }> {
  const SPEC = mcpToolEndpoint(toolName);
  if (SPEC.bucket === null) return { ok: true };
  const limit = await checkAndIncrement(tokenId, SPEC.bucket);
  if (limit.ok) return { ok: true };
  return {
    ok: false,
    result: textResult(
      `Rate limited: daily ${RATE_LIMIT_NOUN[SPEC.bucket]} limit (${limit.limit}) exceeded. Resets at ${limit.resetAt.toISOString()}.`,
      true,
    ),
  };
}
