"use client";

import { useActionState, useState, useTransition } from "react";

import {
  publishPolicyPromptFormAction,
  previewPolicyPromptAction,
  type PolicyPromptActionState,
  type PreviewFixtureResult,
} from "@/lib/actions/policy-prompt";

const INIT: PolicyPromptActionState = { ok: true, message: "" };

interface Props {
  initialSystemPrompt: string;
  suggestedVersion: string;
}

/**
 * Editor for the AI policy moderator's system prompt.
 *
 * Two actions:
 *   - "Preview" runs the draft against four fixture cases (good
 *     submission, spam, doxxing, security-research-as-FP-risk) and
 *     shows whether each scored as expected. Costs ~$0.0005 per
 *     run, takes ~5–10s.
 *   - "Activate" inserts a new row in moderation_prompts with the
 *     given version label and atomically flips it active. The next
 *     moderate() call picks up the new prompt within ~60s (cache
 *     TTL) — the server action also clears the in-process cache so
 *     warm processes pick it up immediately.
 */
export function PolicyPromptEditor({
  initialSystemPrompt,
  suggestedVersion,
}: Props) {
  const [systemPrompt, setSystemPrompt] = useState(initialSystemPrompt);
  const [version, setVersion] = useState(suggestedVersion);
  const [note, setNote] = useState("");
  const [previewResults, setPreviewResults] = useState<PreviewFixtureResult[] | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [previewPending, startPreview] = useTransition();
  const [diffOpen, setDiffOpen] = useState(false);

  const isDirty = systemPrompt !== initialSystemPrompt;

  const [publishState, publishAction, publishPending] = useActionState(
    publishPolicyPromptFormAction,
    INIT,
  );

  function handlePreview() {
    setPreviewError(null);
    setPreviewResults(null);
    startPreview(async () => {
      const result = await previewPolicyPromptAction({ systemPrompt });
      if (result.ok) {
        setPreviewResults(result.results);
      } else {
        setPreviewError(formatPreviewError(result));
      }
    });
  }

  const matchedCount = previewResults
    ? previewResults.filter((r) => r.matched).length
    : 0;
  const totalCount = previewResults?.length ?? 0;
  const previewMismatchWarn =
    previewResults !== null && matchedCount < totalCount;

  return (
    <section className="proto-section">
      <h2>Edit the system prompt</h2>
      <p className="proto-dek">
        Saving creates a new version and activates it immediately. The
        previous version is preserved in history (it just becomes
        inactive). Run <strong>Preview</strong> first — it scores four
        known fixtures with the draft prompt so you can see the
        verdicts before a real submission hits it in production.
      </p>

      <form action={publishAction} className="proto-policy-prompt-form">
        <label htmlFor="systemPrompt" className="proto-form-label">
          System prompt
        </label>
        <textarea
          id="systemPrompt"
          name="systemPrompt"
          value={systemPrompt}
          onChange={(e) => setSystemPrompt(e.target.value)}
          rows={24}
          minLength={200}
          maxLength={16_000}
          required
          disabled={publishPending}
          className="proto-policy-prompt-textarea"
        />
        <p className="proto-form-hint">
          {systemPrompt.length} characters. The user prompt template
          and JSON-schema response format stay in code (changing them
          is a code change). Categories and category descriptions
          live here.
        </p>

        <div className="proto-form-row">
          <label htmlFor="version" className="proto-form-label">
            Version label
          </label>
          <input
            id="version"
            name="version"
            value={version}
            onChange={(e) => setVersion(e.target.value)}
            required
            disabled={publishPending}
            pattern="[A-Za-z0-9_.\-]+"
            maxLength={40}
            className="proto-form-input"
          />
        </div>

        <div className="proto-form-row">
          <label htmlFor="note" className="proto-form-label">
            Note (optional)
          </label>
          <input
            id="note"
            name="note"
            value={note}
            onChange={(e) => setNote(e.target.value)}
            maxLength={500}
            placeholder="e.g. tighten doxxing definition; loosen spam threshold"
            disabled={publishPending}
            className="proto-form-input"
          />
        </div>

        <div className="proto-form-actions">
          <button
            type="button"
            onClick={handlePreview}
            disabled={previewPending || systemPrompt.length < 200}
            className="proto-btn"
          >
            {previewPending ? "Previewing…" : "Preview"}
          </button>
          <button
            type="button"
            onClick={() => setDiffOpen((v) => !v)}
            disabled={!isDirty}
            className="proto-btn"
            aria-pressed={diffOpen}
            title={
              isDirty
                ? "Compare your edit against the currently-active prompt"
                : "No changes to diff yet"
            }
          >
            {diffOpen ? "Hide diff" : "Show diff vs active"}
          </button>
          <button
            type="submit"
            disabled={publishPending}
            className="proto-btn proto-btn-primary"
          >
            {publishPending ? "Activating…" : "Activate"}
          </button>
          {publishState.message ? (
            <span
              className={
                publishState.ok
                  ? "proto-form-flash proto-form-flash-ok"
                  : "proto-form-flash proto-form-flash-err"
              }
              role={publishState.ok ? "status" : "alert"}
            >
              {publishState.message}
            </span>
          ) : null}
        </div>
      </form>

      {diffOpen && isDirty ? (
        <PromptDiff
          activeText={initialSystemPrompt}
          editText={systemPrompt}
        />
      ) : null}

      {previewMismatchWarn ? (
        <p className="proto-form-flash proto-form-flash-err" role="alert">
          ⚠ Preview: {totalCount - matchedCount}/{totalCount} fixtures
          scored differently than expected. Review before activating.
        </p>
      ) : null}

      {previewError ? (
        <p className="proto-form-flash proto-form-flash-err" role="alert">
          {previewError}
        </p>
      ) : null}

      {previewResults ? (
        <table className="proto-mod-table">
          <caption>
            Preview: {matchedCount}/{totalCount} fixtures matched expected.
          </caption>
          <thead>
            <tr>
              <th>Fixture</th>
              <th>Expected</th>
              <th>Actual</th>
              <th>Confidence</th>
              <th>One-line-why</th>
              <th>ms</th>
            </tr>
          </thead>
          <tbody>
            {previewResults.map((r) => {
              const actualLabel = r.actual
                ? r.actual.verdict === "reject"
                  ? `reject:${r.actual.category}`
                  : "pass"
                : "(error)";
              return (
                <tr key={r.label}>
                  <td>{r.label}</td>
                  <td>
                    <code>{r.expected}</code>
                  </td>
                  <td>
                    <code
                      className={
                        r.matched
                          ? "proto-state-pill proto-state-pill-pending"
                          : "proto-state-pill proto-state-pill-pending proto-mod-btn-remove"
                      }
                    >
                      {actualLabel}
                    </code>
                  </td>
                  <td>{r.actual?.confidence ?? "—"}</td>
                  <td className="proto-mod-reason">
                    {r.actual?.oneLineWhy ?? r.error ?? "—"}
                  </td>
                  <td>{r.elapsedMs}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      ) : null}
    </section>
  );
}

function formatPreviewError(
  result: { reason: "forbidden" | "validation" | "no_api_key"; detail?: string },
): string {
  switch (result.reason) {
    case "forbidden":
      return "Not authorized.";
    case "validation":
      return result.detail ?? "Invalid input.";
    case "no_api_key":
      return "OPENAI_API_KEY is not set in this environment — preview cannot run.";
  }
}

/**
 * Side-by-side prompt diff. Naive line-equality check —
 * "active" lines that don't exist verbatim in the edit buffer
 * are tinted as removed; "edit" lines that don't exist verbatim
 * in the active buffer are tinted as added; lines present on
 * both sides render muted. This is not a Myers diff, but for a
 * 200–8000-char system prompt the operator is scanning for
 * directional change, not edit-distance optimality.
 *
 * Counts at the top tell the operator the magnitude of the
 * edit at a glance; the side-by-side panel tells them where.
 */
function PromptDiff({
  activeText,
  editText,
}: {
  activeText: string;
  editText: string;
}) {
  const activeLines = activeText.split("\n");
  const editLines = editText.split("\n");
  const activeSet = new Set(activeLines);
  const editSet = new Set(editLines);
  const added = editLines.filter((l) => !activeSet.has(l)).length;
  const removed = activeLines.filter((l) => !editSet.has(l)).length;

  return (
    <section className="proto-section proto-prompt-diff">
      <div className="proto-prompt-diff-summary">
        <span className="proto-prompt-diff-count proto-prompt-diff-removed">
          −{removed}
        </span>
        <span className="proto-prompt-diff-count proto-prompt-diff-added">
          +{added}
        </span>
        <span className="proto-meta-quiet">
          line-equality diff vs the currently-active prompt
        </span>
      </div>
      <div className="proto-prompt-diff-grid">
        <div>
          <h4 className="proto-prompt-diff-head">Active</h4>
          <pre className="proto-prompt-diff-pane">
            {activeLines.map((line, i) => (
              <span
                key={i}
                className={
                  editSet.has(line)
                    ? "proto-prompt-diff-line"
                    : "proto-prompt-diff-line proto-prompt-diff-line-removed"
                }
              >
                {line || " "}
                {"\n"}
              </span>
            ))}
          </pre>
        </div>
        <div>
          <h4 className="proto-prompt-diff-head">Edit buffer</h4>
          <pre className="proto-prompt-diff-pane">
            {editLines.map((line, i) => (
              <span
                key={i}
                className={
                  activeSet.has(line)
                    ? "proto-prompt-diff-line"
                    : "proto-prompt-diff-line proto-prompt-diff-line-added"
                }
              >
                {line || " "}
                {"\n"}
              </span>
            ))}
          </pre>
        </div>
      </div>
    </section>
  );
}
