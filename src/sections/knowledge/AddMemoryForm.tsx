// Manual knowledge authoring — the deliberately-secondary intake.
//
// The pipeline (Review) is the primary way knowledge enters the base; the
// distiller proposes and the human judges. This form exists only because
// the old flat Memories and Decisions tabs had create/log affordances.
// It sits behind a single non-primary "Add" toggle (knowledge-base-pane.md
// §5.3), never a primary action. Manually authored records are already
// human-gated: memories land `accepted`, and decisions land `active`.

import { useCallback, useState } from "react";
import { sharedMemoryApi } from "../../api/sharedMemory";
import type { MemoryKind, MemoryScope } from "../../api/sharedMemory";
import { Button } from "../../components/primitives/Button";
import { Input } from "../../components/primitives/Input";
import { toUserError } from "../../lib/errors";

export function AddMemoryForm({
  defaultProject,
  knownProjects = [],
  onCreated,
  onCancel,
}: {
  /** Pre-fill the project path when the view is filtered to one project. */
  defaultProject?: string;
  /** Known project paths, offered as autocomplete so a hand-typed path
   *  can't silently orphan a record under a project that doesn't exist. */
  knownProjects?: string[];
  onCreated: () => void;
  onCancel: () => void;
}) {
  const [scope, setScope] = useState<MemoryScope>(
    defaultProject ? "project" : "global",
  );
  const [mode, setMode] = useState<"memory" | "decision">("memory");
  const [projectPath, setProjectPath] = useState(defaultProject ?? "");
  const [kind, setKind] = useState<MemoryKind>("fact");
  const [content, setContent] = useState("");
  const [topic, setTopic] = useState("");
  const [rationale, setRationale] = useState("");
  const [createdBy, setCreatedBy] = useState("user:me");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const submit = useCallback(async () => {
    if (!content.trim() || !createdBy.trim()) return;
    if (scope === "project" && !projectPath.trim()) {
      setErr("A project path is required for project scope.");
      return;
    }
    setBusy(true);
    setErr(null);
    try {
      if (mode === "memory") {
        await sharedMemoryApi.createMemory({
          scope,
          project_path: scope === "project" ? projectPath.trim() : null,
          kind,
          content: content.trim(),
          created_by: createdBy.trim(),
        });
      } else {
        await sharedMemoryApi.logDecision({
          decision: content.trim(),
          rationale: rationale.trim() || null,
          topic: topic.trim() || null,
          project_path: scope === "project" ? projectPath.trim() : null,
          created_by: createdBy.trim(),
        });
      }
      setContent("");
      setTopic("");
      setRationale("");
      onCreated();
    } catch (e) {
      setErr(toUserError(e));
    } finally {
      setBusy(false);
    }
  }, [mode, scope, projectPath, kind, content, topic, rationale, createdBy, onCreated]);

  return (
    <div
      style={{
        border: "var(--sp-px) solid var(--line)",
        borderRadius: "var(--r-3)",
        padding: "var(--sp-16)",
        background: "var(--bg-raised)",
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-10)",
      }}
    >
      <div style={{ display: "flex", gap: "var(--sp-8)" }}>
        <select
          value={mode}
          onChange={(e) => setMode(e.currentTarget.value as "memory" | "decision")}
          aria-label="Knowledge type"
          style={selectStyle()}
        >
          <option value="memory">Memory</option>
          <option value="decision">Decision</option>
        </select>
        <select
          value={scope}
          onChange={(e) => setScope(e.currentTarget.value as MemoryScope)}
          aria-label="Scope"
          style={selectStyle()}
        >
          <option value="global">Global</option>
          <option value="project">Project</option>
        </select>
        {mode === "memory" ? (
          <select
            value={kind}
            onChange={(e) => setKind(e.currentTarget.value as MemoryKind)}
            aria-label="Kind"
            style={selectStyle()}
          >
            <option value="fact">Fact</option>
            <option value="preference">Preference</option>
            <option value="pattern">Pattern</option>
            <option value="constraint">Constraint</option>
            <option value="summary">Summary</option>
          </select>
        ) : (
          <Input
            value={topic}
            onChange={(e) => setTopic(e.currentTarget.value)}
            placeholder="topic (optional)"
            aria-label="Decision topic"
            style={{ flex: 1 }}
          />
        )}
        {scope === "project" && (
          <Input
            value={projectPath}
            onChange={(e) => setProjectPath(e.currentTarget.value)}
            placeholder="project path (absolute)"
            aria-label="Project path"
            list="add-memory-projects"
            style={{ flex: 1 }}
          />
        )}
        {scope === "project" && knownProjects.length > 0 && (
          <datalist id="add-memory-projects">
            {knownProjects.map((p) => (
              <option key={p} value={p} />
            ))}
          </datalist>
        )}
      </div>
      <textarea
        value={content}
        onChange={(e) => setContent(e.currentTarget.value)}
        placeholder={mode === "memory" ? "What should we remember?" : "What decision should this record?"}
        aria-label={mode === "memory" ? "Memory content" : "Decision"}
        rows={3}
        style={{
          padding: "var(--sp-8)",
          background: "var(--bg-sunken)",
          color: "var(--fg)",
          border: "var(--sp-px) solid var(--line)",
          borderRadius: "var(--r-2)",
          font: "inherit",
          resize: "vertical",
        }}
      />
      {mode === "decision" && (
        <textarea
          value={rationale}
          onChange={(e) => setRationale(e.currentTarget.value)}
          placeholder="Why? (optional)"
          aria-label="Decision rationale"
          rows={2}
          style={{
            padding: "var(--sp-8)",
            background: "var(--bg-sunken)",
            color: "var(--fg)",
            border: "var(--sp-px) solid var(--line)",
            borderRadius: "var(--r-2)",
            font: "inherit",
            resize: "vertical",
          }}
        />
      )}
      <div style={{ display: "flex", gap: "var(--sp-8)" }}>
        <Input
          value={createdBy}
          onChange={(e) => setCreatedBy(e.currentTarget.value)}
          placeholder="created_by (e.g. user:me)"
          aria-label="Created by"
          style={{ flex: 1 }}
        />
        <Button onClick={onCancel} disabled={busy}>
          Cancel
        </Button>
        <Button
          variant="solid"
          onClick={() => void submit()}
          disabled={busy || !content.trim()}
        >
          {busy ? "Saving…" : mode === "memory" ? "Save memory" : "Save decision"}
        </Button>
      </div>
      {err && <div style={{ color: "var(--danger)", fontSize: "var(--fs-sm)" }}>{err}</div>}
    </div>
  );
}

function selectStyle(): React.CSSProperties {
  return {
    padding: "0 var(--sp-8)",
    height: "var(--input-height)",
    background: "var(--bg-raised)",
    color: "var(--fg)",
    border: "var(--sp-px) solid var(--line)",
    borderRadius: "var(--r-2)",
    font: "inherit",
  };
}
