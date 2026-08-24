/**
 * Settings → Quick prompts — CRUD for the chips above the remote
 * panel's composer.
 *
 * The panel shipped four hardcoded strings, which is the right list for
 * nobody in particular: the useful ones are the phrases a given person
 * types twenty times a week, and those are not knowable from here.
 *
 * **Edited as a whole list, saved as a whole list.** Order is part of
 * the data, so a per-row save would need a reorder verb doing the same
 * job. The pane holds a draft and saves it in one call — which is also
 * why there is an explicit Save: a chip that fires on the phone is not
 * something to change on every keystroke.
 */
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { quickPromptApi } from "../../api";
import { Button } from "../../components/primitives/Button";
import { IconButton } from "../../components/primitives/IconButton";
import { NF } from "../../icons";
import {
  QUICK_PROMPT_MAX_COUNT,
  QUICK_PROMPT_MAX_NAME,
  QUICK_PROMPT_MAX_TEXT,
  type QuickPrompt,
} from "../../types";

function newId(): string {
  return globalThis.crypto?.randomUUID?.() ?? String(Date.now() + Math.random());
}

export function QuickPromptsPane({
  pushToast,
}: {
  pushToast?: (kind: "info" | "error", text: string) => void;
}) {
  const { t } = useTranslation("settings");
  const [draft, setDraft] = useState<QuickPrompt[] | null>(null);
  const [saved, setSaved] = useState<QuickPrompt[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [loadError, setLoadError] = useState(false);

  const load = useCallback(() => {
    quickPromptApi
      .list()
      .then((p) => {
        setDraft(p);
        setSaved(p);
      })
      .catch(() => setLoadError(true));
  }, []);
  useEffect(load, [load]);

  // Compared by value, so reordering two rows and putting them back
  // counts as no change — the alternative is a Save button that stays
  // lit because something was touched.
  const dirty = draft !== null && JSON.stringify(draft) !== JSON.stringify(saved);

  const update = (i: number, patch: Partial<QuickPrompt>) =>
    setDraft((d) => d?.map((p, n) => (n === i ? { ...p, ...patch } : p)) ?? d);

  const move = (i: number, by: number) =>
    setDraft((d) => {
      if (!d) return d;
      const j = i + by;
      if (j < 0 || j >= d.length) return d;
      const next = [...d];
      [next[i], next[j]] = [next[j], next[i]];
      return next;
    });

  /**
   * Restore the built-in four.
   *
   * `await`ed with a `catch`, not `.then(setDraft)`. Fire-and-forget
   * turned an IPC failure into an unhandled rejection and left the user
   * looking at an unchanged list with nothing to explain why — the same
   * silent-tap failure the panel's send path was fixed for.
   */
  const restore = async () => {
    setBusy(true);
    try {
      setDraft(await quickPromptApi.defaults());
    } catch (e) {
      const detail =
        typeof e === "object" && e && "message" in e ? String(e.message) : String(e);
      pushToast?.("error", `${t("quickPrompts.restoreFailed")}: ${detail}`);
    } finally {
      setBusy(false);
    }
  };

  const save = async () => {
    if (!draft) return;
    setBusy(true);
    try {
      const stored = await quickPromptApi.save(draft);
      setDraft(stored);
      setSaved(stored);
      pushToast?.("info", t("quickPrompts.saved"));
    } catch (e) {
      // The core error names the rule that was broken — a duplicate
      // name, an empty field, too many. Passing it through beats
      // "save failed", which sends the user hunting.
      const detail =
        typeof e === "object" && e && "message" in e ? String(e.message) : String(e);
      pushToast?.("error", `${t("quickPrompts.saveFailed")}: ${detail}`);
    } finally {
      setBusy(false);
    }
  };

  if (loadError) return <p className="muted">{t("quickPrompts.loadFailed")}</p>;
  if (!draft) return <p className="muted">…</p>;

  return (
    <div>
      <p className="muted">{t("quickPrompts.intro")}</p>

      {draft.length === 0 && <p className="muted">{t("quickPrompts.empty")}</p>}

      {draft.map((p, i) => (
        <div key={p.id} className="qp-row">
          <div className="qp-fields">
            <input
              aria-label={t("quickPrompts.name")}
              placeholder={t("quickPrompts.namePlaceholder")}
              value={p.name}
              maxLength={QUICK_PROMPT_MAX_NAME}
              onChange={(e) => update(i, { name: e.target.value })}
            />
            <textarea
              aria-label={t("quickPrompts.text")}
              placeholder={t("quickPrompts.textPlaceholder")}
              value={p.text}
              rows={2}
              maxLength={QUICK_PROMPT_MAX_TEXT}
              onChange={(e) => update(i, { text: e.target.value })}
            />
          </div>
          <div className="qp-actions">
            <IconButton
              glyph={NF.chevronU}
              title={t("quickPrompts.moveUp")}
              aria-label={t("quickPrompts.moveUp")}
              disabled={i === 0}
              onClick={() => move(i, -1)}
            />
            <IconButton
              glyph={NF.chevronD}
              title={t("quickPrompts.moveDown")}
              aria-label={t("quickPrompts.moveDown")}
              disabled={i === draft.length - 1}
              onClick={() => move(i, 1)}
            />
            <IconButton
              glyph={NF.trash}
              title={t("quickPrompts.remove")}
              aria-label={t("quickPrompts.remove")}
              onClick={() => setDraft((d) => d?.filter((_, n) => n !== i) ?? d)}
            />
          </div>
        </div>
      ))}

      <div className="qp-footer">
        <Button
          variant="subtle"
          glyph={NF.plus}
          disabled={draft.length >= QUICK_PROMPT_MAX_COUNT}
          onClick={() => setDraft((d) => [...(d ?? []), { id: newId(), name: "", text: "" }])}
        >
          {t("quickPrompts.add")}
        </Button>
        <Button variant="ghost" disabled={busy} onClick={restore}>
          {t("quickPrompts.restore")}
        </Button>
        <span className="muted">
          {t("quickPrompts.count", { count: draft.length, max: QUICK_PROMPT_MAX_COUNT })}
        </span>
        <Button variant="solid" disabled={!dirty || busy} onClick={save}>
          {t("quickPrompts.save")}
        </Button>
        {/* Inline, not a tooltip: per `rules/design.md` a disabled
            control states its reason next to itself. */}
        {dirty && <span className="muted">{t("quickPrompts.unsaved")}</span>}
      </div>
    </div>
  );
}
