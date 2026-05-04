import { query } from "@anthropic-ai/claude-agent-sdk";

const MODEL = "claude-sonnet-4-6";

/**
 * Score a submission via the Claude Agent SDK using the user's Claude Code OAuth.
 *
 * No ANTHROPIC_API_KEY needed — auth flows through the local Claude Code CLI session.
 * Requires Claude Code CLI installed and the user logged in (`claude /login`).
 *
 * Returns the parsed JSON response (un-validated; caller runs Zod parse so
 * validation lives in one place — score.ts).
 */
export async function scoreWithClaude(
  systemPrompt: string,
  userPrompt: string
): Promise<unknown> {
  const debug = process.env.EDITORIAL_DEBUG === "1";

  for await (const msg of query({
    prompt: userPrompt,
    options: {
      model: MODEL,
      systemPrompt,
      allowedTools: [],          // pure scoring — no file/bash/web tool access
      disallowedTools: [
        "Bash", "Read", "Write", "Edit", "Glob", "Grep", "WebFetch", "WebSearch",
        "Task", "TodoWrite", "Skill", "AskUserQuestion",
      ],
      settingSources: [],        // ignore user/project/local settings (skills, hooks, plugins)
      persistSession: false,     // don't pollute ~/.claude/projects/ with scoring runs
      maxTurns: 1,               // single turn — model thinks then emits JSON
    },
  })) {
    if (debug) {
      const summary = JSON.stringify(msg).slice(0, 400);
      console.error(`[debug] ${msg.type}${"subtype" in msg ? `/${msg.subtype}` : ""}: ${summary}`);
    }
    if (msg.type === "result") {
      if (msg.subtype === "success") {
        return parseJsonResponse(msg.result ?? "");
      }
      throw new Error(`Claude scoring failed (${msg.subtype}): ${msg.errors.join(" | ")}`);
    }
  }
  throw new Error("Claude Agent SDK exited without a result message");
}

/**
 * Extract the first JSON object from a model response.
 * Handles bare JSON and markdown-fenced JSON (```json ... ``` or ``` ... ```).
 * Throws with the head of the response if no JSON object is found.
 */
function parseJsonResponse(raw: string): unknown {
  const trimmed = raw.trim();

  const fenced = trimmed.match(/```(?:json)?\s*\n([\s\S]*?)\n```/);
  const candidate = fenced ? fenced[1] : trimmed;

  const objectMatch = candidate.match(/\{[\s\S]*\}/);
  if (!objectMatch) {
    throw new Error(
      `No JSON object in Claude response. First 300 chars: ${raw.slice(0, 300)}`
    );
  }

  try {
    return JSON.parse(objectMatch[0]);
  } catch (err) {
    throw new Error(
      `Failed to parse JSON from Claude response: ${err instanceof Error ? err.message : String(err)}\nFirst 300 chars: ${raw.slice(0, 300)}`
    );
  }
}
