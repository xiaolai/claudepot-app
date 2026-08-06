// Manual knowledge authoring — the deliberately-secondary intake.
//
// The pipeline (Review) is the primary way knowledge enters the base; the
// distiller proposes and the human judges. This form exists only because
// the old flat Memories and Decisions tabs had create/log affordances.
// It sits behind a single non-primary "Add" toggle (knowledge-base-pane.md
// §5.3), never a primary action. Manually authored records are already
// human-gated: memories land `accepted`, and decisions land `active`.

import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { sharedMemoryApi } from "../../api/sharedMemory";
import type { MemoryKind, MemoryScope } from "../../api/sharedMemory";
import { Button } from "../../components/primitives/Button";
import { Input } from "../../components/primitives/Input";
import { renderError } from "../../lib/i18n-error";

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
  const { t } = useTranslation("knowledge");
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
      setErr(t("know.add.errProjectRequired"));
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
      setErr(renderError(e));
    } finally {
      setBusy(false);
    }
  }, [mode, scope, projectPath, kind, content, topic, rationale, createdBy, onCreated, t]);

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
          aria-label={t("know.add.typeAria")}
          style={selectStyle()}
        >
          <option value="memory">{t("know.add.typeMemory")}</option>
          <option value="decision">{t("know.add.typeDecision")}</option>
        </select>
        <select
          value={scope}
          onChange={(e) => setScope(e.currentTarget.value as MemoryScope)}
          aria-label={t("know.add.scopeAria")}
          style={selectStyle()}
        >
          <option value="global">{t("know.add.scopeGlobal")}</option>
          <option value="project">{t("know.add.scopeProject")}</option>
        </select>
        {mode === "memory" ? (
          <select
            value={kind}
            onChange={(e) => setKind(e.currentTarget.value as MemoryKind)}
            aria-label={t("know.add.kindAria")}
            style={selectStyle()}
          >
            <option value="fact">{t("know.add.kindFact")}</option>
            <option value="preference">{t("know.add.kindPreference")}</option>
            <option value="pattern">{t("know.add.kindPattern")}</option>
            <option value="constraint">{t("know.add.kindConstraint")}</option>
            <option value="summary">{t("know.add.kindSummary")}</option>
          </select>
        ) : (
          <Input
            value={topic}
            onChange={(e) => setTopic(e.currentTarget.value)}
            placeholder={t("know.add.topicPlaceholder")}
            aria-label={t("know.add.topicAria")}
            style={{ flex: 1 }}
          />
        )}
        {scope === "project" && (
          <Input
            value={projectPath}
            onChange={(e) => setProjectPath(e.currentTarget.value)}
            placeholder={t("know.add.projectPlaceholder")}
            aria-label={t("know.add.projectAria")}
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
        placeholder={
          mode === "memory"
            ? t("know.add.contentPlaceholderMemory")
            : t("know.add.contentPlaceholderDecision")
        }
        aria-label={
          mode === "memory"
            ? t("know.add.contentAriaMemory")
            : t("know.add.contentAriaDecision")
        }
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
          placeholder={t("know.add.rationalePlaceholder")}
          aria-label={t("know.add.rationaleAria")}
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
          placeholder={t("know.add.createdByPlaceholder")}
          aria-label={t("know.add.createdByAria")}
          style={{ flex: 1 }}
        />
        <Button onClick={onCancel} disabled={busy}>
          {t("know.add.cancel")}
        </Button>
        <Button
          variant="solid"
          onClick={() => void submit()}
          disabled={busy || !content.trim()}
        >
          {busy
            ? t("know.add.saving")
            : mode === "memory"
              ? t("know.add.saveMemory")
              : t("know.add.saveDecision")}
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
