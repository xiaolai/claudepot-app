// Manual memory authoring — the deliberately-secondary intake.
//
// The pipeline (Review) is the primary way knowledge enters the base; the
// distiller proposes and the human judges. This form exists only because
// the old flat Memories tab had a create affordance and dropping it would
// lose a capability. It sits behind a single non-primary "Add" toggle
// (knowledge-base-pane.md §5.3), never a primary action. A manually
// authored memory lands `accepted` — a human wrote it, so it is not a
// proposal awaiting review.

import { useCallback, useState } from "react";
import { sharedMemoryApi } from "../../api/sharedMemory";
import type { MemoryKind, MemoryScope } from "../../api/sharedMemory";
import { Button } from "../../components/primitives/Button";
import { Input } from "../../components/primitives/Input";

export function AddMemoryForm({
  defaultProject,
  onCreated,
  onCancel,
}: {
  /** Pre-fill the project path when the view is filtered to one project. */
  defaultProject?: string;
  onCreated: () => void;
  onCancel: () => void;
}) {
  const [scope, setScope] = useState<MemoryScope>(
    defaultProject ? "project" : "global",
  );
  const [projectPath, setProjectPath] = useState(defaultProject ?? "");
  const [kind, setKind] = useState<MemoryKind>("fact");
  const [content, setContent] = useState("");
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
      await sharedMemoryApi.createMemory({
        scope,
        project_path: scope === "project" ? projectPath.trim() : null,
        kind,
        content: content.trim(),
        created_by: createdBy.trim(),
      });
      setContent("");
      onCreated();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }, [scope, projectPath, kind, content, createdBy, onCreated]);

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
          value={scope}
          onChange={(e) => setScope(e.currentTarget.value as MemoryScope)}
          aria-label="Scope"
          style={selectStyle()}
        >
          <option value="global">Global</option>
          <option value="project">Project</option>
        </select>
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
        {scope === "project" && (
          <Input
            value={projectPath}
            onChange={(e) => setProjectPath(e.currentTarget.value)}
            placeholder="project path (absolute)"
            aria-label="Project path"
            style={{ flex: 1 }}
          />
        )}
      </div>
      <textarea
        value={content}
        onChange={(e) => setContent(e.currentTarget.value)}
        placeholder="What should we remember?"
        aria-label="Memory content"
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
          {busy ? "Saving…" : "Save"}
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
