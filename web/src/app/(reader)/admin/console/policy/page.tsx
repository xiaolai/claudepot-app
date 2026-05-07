import Link from "next/link";

import { staffGate } from "@/lib/staff-gate";
import { relativeTime } from "@/lib/format";
import { loadPolicyPromptHistory } from "@/lib/actions/policy-prompt";
import { FALLBACK_SYSTEM_PROMPT } from "@/lib/moderation/prompt";

import { PolicyPromptEditor } from "./PolicyPromptEditor";

/**
 * /admin/policy-prompt — staff editor for the AI policy moderator's
 * system prompt.
 *
 * Today the moderator is Ada (per migration 0009 + the system-user
 * lookup). Saving here creates a new row in moderation_prompts with
 * active=true and atomically deactivates the prior active row. Old
 * versions stay in history; "rollback" is just creating a new row
 * with the old content.
 *
 * The fallback (FALLBACK_SYSTEM_PROMPT in lib/moderation/prompt.ts)
 * is the boot-time default when the DB table is empty. Once a row
 * exists, the DB version always wins.
 */

export default async function PolicyPromptAdminPage({
  searchParams,
}: {
  searchParams: Promise<{ as?: string }>;
}) {
  const sp = await searchParams;
  const gate = await staffGate(sp);
  if (gate) return gate;

  const history = await loadPolicyPromptHistory();
  const active = history.find((h) => h.active);
  const initialSystemPrompt = active?.systemPrompt ?? FALLBACK_SYSTEM_PROMPT;

  // Suggest the next integer if the version label has been integers
  // so far; otherwise echo "fallback+1" / fall back to a date stamp.
  const suggestedVersion = suggestNextVersion(history.map((h) => h.version));

  return (
    <div className="proto-page-narrow">
      <h1>Policy moderator prompt</h1>
      <p className="proto-dek">
        The system prompt the AI policy moderator (
        <Link href="/office/persona/ada">Ada</Link>) uses to score
        every submission and comment. Edits here take effect within
        ~60s without a redeploy. Past versions stay in history and
        can be rolled back by saving their content as a new version.
      </p>

      <section className="proto-section">
        <h2>Currently active</h2>
        {active ? (
          <p className="proto-dek">
            <strong>{active.version}</strong>
            {active.note ? <> — {active.note}</> : null} · saved{" "}
            {relativeTime(active.createdAt.toISOString())}
            {active.createdByUsername ? (
              <> by @{active.createdByUsername}</>
            ) : null}
            .
          </p>
        ) : (
          <p className="proto-dek">
            No DB row active yet — moderator is using the fallback
            prompt baked into the deploy. Save a version below to
            switch the moderator to DB-backed prompts.
          </p>
        )}
      </section>

      <PolicyPromptEditor
        initialSystemPrompt={initialSystemPrompt}
        suggestedVersion={suggestedVersion}
      />

      <section className="proto-section">
        <h2>History</h2>
        {history.length === 0 ? (
          <p className="proto-empty">No saved versions yet.</p>
        ) : (
          <table className="proto-mod-table">
            <thead>
              <tr>
                <th>Version</th>
                <th>Saved</th>
                <th>By</th>
                <th>Active</th>
                <th>Note</th>
              </tr>
            </thead>
            <tbody>
              {history.map((h) => (
                <tr key={h.id}>
                  <td>
                    <code>{h.version}</code>
                  </td>
                  <td>{relativeTime(h.createdAt.toISOString())}</td>
                  <td>
                    {h.createdByUsername ? (
                      <Link href={`/u/${h.createdByUsername}`}>
                        @{h.createdByUsername}
                      </Link>
                    ) : (
                      "—"
                    )}
                  </td>
                  <td>{h.active ? "✓" : ""}</td>
                  <td className="proto-mod-reason">{h.note ?? ""}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </div>
  );
}

/**
 * Return a sensible default for the version input. If every existing
 * version is a parseable integer, suggest max+1. Otherwise suggest
 * an ISO-date stamp so the user never gets a duplicate by accident.
 */
function suggestNextVersion(existing: string[]): string {
  const ints: number[] = [];
  let allInts = true;
  for (const v of existing) {
    const n = Number.parseInt(v, 10);
    if (Number.isInteger(n) && String(n) === v) {
      ints.push(n);
    } else {
      allInts = false;
      break;
    }
  }
  if (allInts && ints.length > 0) {
    return String(Math.max(...ints) + 1);
  }
  // Fallback: a date stamp (always unique among recent edits).
  const now = new Date();
  const yyyy = now.getUTCFullYear();
  const mm = String(now.getUTCMonth() + 1).padStart(2, "0");
  const dd = String(now.getUTCDate()).padStart(2, "0");
  return `${yyyy}-${mm}-${dd}`;
}
