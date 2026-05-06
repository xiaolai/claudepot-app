/**
 * Tiny helpers shared across the per-domain MCP tool modules.
 */

import type { z } from "zod";

export function textResult(text: string, isError = false) {
  return { isError, content: [{ type: "text" as const, text }] };
}

// Format a Zod safeParse error into the inline text the tool result
// surfaces back to the LLM client.
export function formatZodIssues(error: z.ZodError): string {
  return error.issues
    .map((i) => `${i.path.join(".") || "<root>"}: ${i.message}`)
    .join("; ");
}
