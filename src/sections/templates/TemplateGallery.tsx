import { useEffect, useMemo, useState } from "react";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import { Modal } from "../../components/primitives/Modal";
import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";
import type {
  TemplateCategory,
  TemplateSummaryDto,
} from "../../types";
import { TemplateInstallDialog } from "./TemplateInstallDialog";

const CATEGORY_LABELS: Record<TemplateCategory, string> = {
  "it-health": "IT health",
  diagnostics: "Diagnostics",
  housekeeping: "Housekeeping",
  audit: "Audit",
  caregiver: "Caregiver",
  network: "Network",
};

interface Props {
  open: boolean;
  onClose: () => void;
  onInstalled: () => void;
  onError: (msg: string) => void;
  onOpenThirdParties: () => void;
}

/**
 * Card-grid modal of all bundled templates. Categories filter at
 * the top. Click a card → `TemplateInstallDialog` mounts in
 * place of the gallery (we hide the gallery beneath rather than
 * stacking modals — single-modal-at-a-time keeps focus
 * unambiguous).
 */
export function TemplateGallery({
  open,
  onClose,
  onInstalled,
  onError,
  onOpenThirdParties,
}: Props) {
  const [templates, setTemplates] = useState<TemplateSummaryDto[] | null>(null);
  const [filter, setFilter] = useState<TemplateCategory | "all">("all");
  const [installTarget, setInstallTarget] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setTemplates(null);
    api
      .templatesList()
      .then(setTemplates)
      .catch((e: unknown) => onError(String(e)));
  }, [open, onError]);

  const visible = useMemo(
    () =>
      (templates ?? []).filter((t) => filter === "all" || t.category === filter),
    [templates, filter],
  );

  const categories = useMemo(() => {
    if (!templates) return [];
    const seen = new Set<TemplateCategory>();
    for (const t of templates) seen.add(t.category);
    return Array.from(seen).sort();
  }, [templates]);

  if (!open) return null;

  // Suppress the gallery while the install dialog is open so
  // focus stays unambiguous and the install dialog is the
  // visually-active surface.
  const galleryHidden = installTarget !== null;

  return (
    <>
      <Modal
        open={open && !galleryHidden}
        onClose={onClose}
        width="lg"
        aria-labelledby="template-gallery-title"
      >
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: "var(--sp-12)",
            padding: "var(--sp-16) var(--sp-20)",
          }}
        >
          <div
            style={{
              display: "flex",
              justifyContent: "space-between",
              alignItems: "center",
              gap: "var(--sp-12)",
            }}
          >
            <h2
              id="template-gallery-title"
              style={{ margin: 0, fontSize: "var(--fs-lg)", color: "var(--fg)" }}
            >
              Install from template
            </h2>
            <Button variant="ghost" onClick={onClose}>
              Close
            </Button>
          </div>

          {categories.length > 1 && (
            <div
              style={{
                display: "flex",
                flexWrap: "wrap",
                gap: "var(--sp-6)",
                fontSize: "var(--fs-xs)",
              }}
            >
              <FilterChip
                active={filter === "all"}
                onClick={() => setFilter("all")}
              >
                All
              </FilterChip>
              {categories.map((c) => (
                <FilterChip
                  key={c}
                  active={filter === c}
                  onClick={() => setFilter(c)}
                >
                  {CATEGORY_LABELS[c]}
                </FilterChip>
              ))}
            </div>
          )}

          {templates === null ? (
            <div
              style={{
                padding: "var(--sp-24)",
                color: "var(--fg-faint)",
                fontSize: "var(--fs-sm)",
              }}
            >
              Loading templates…
            </div>
          ) : visible.length === 0 ? (
            <div
              style={{
                padding: "var(--sp-24)",
                color: "var(--fg-faint)",
                fontSize: "var(--fs-sm)",
                textAlign: "center",
              }}
            >
              No templates in this category.
            </div>
          ) : (
            <div
              style={{
                display: "grid",
                gridTemplateColumns: "repeat(auto-fill, minmax(tokens.config.menu.min.width, 1fr))",
                gap: "var(--sp-12)",
              }}
            >
              {visible.map((t) => (
                <TemplateCard
                  key={t.id}
                  template={t}
                  onClick={() => setInstallTarget(t.id)}
                />
              ))}
            </div>
          )}
        </div>
      </Modal>

      <TemplateInstallDialog
        open={installTarget !== null}
        templateId={installTarget}
        onClose={() => setInstallTarget(null)}
        onInstalled={() => {
          setInstallTarget(null);
          onInstalled();
        }}
        onError={onError}
        onOpenThirdParties={onOpenThirdParties}
      />
    </>
  );
}

function TemplateCard({
  template,
  onClick,
}: {
  template: TemplateSummaryDto;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      style={{
        textAlign: "left",
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-8)",
        padding: "var(--sp-12) var(--sp-14)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        background: "var(--bg-raised)",
        color: "var(--fg)",
        cursor: "pointer",
        font: "inherit",
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-8)",
        }}
      >
        <Glyph g={NF.dot} />
        <span
          style={{
            fontSize: "var(--fs-md)",
            color: "var(--fg)",
          }}
        >
          {template.name}
        </span>
      </div>
      <p
        style={{
          margin: 0,
          fontSize: "var(--fs-sm)",
          color: "var(--fg-2)",
          lineHeight: 1.45,
        }}
      >
        {template.tagline}
      </p>
      <div
        style={{
          display: "flex",
          flexWrap: "wrap",
          gap: "var(--sp-6)",
          marginTop: "auto",
          paddingTop: "var(--sp-6)",
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-faint)",
        }}
        className="mono-cap"
      >
        <Tag>{CATEGORY_LABELS[template.category]}</Tag>
        <Tag>{template.default_schedule_label}</Tag>
        <Tag>{template.cost_class}</Tag>
        {template.privacy === "local" && <Tag tone="accent">local-only</Tag>}
      </div>
    </button>
  );
}

function Tag({
  children,
  tone = "neutral",
}: {
  children: React.ReactNode;
  tone?: "neutral" | "accent";
}) {
  return (
    <span
      style={{
        padding: "tokens.sp.px tokens.sp[6]",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-1)",
        background: tone === "accent" ? "var(--accent-soft)" : "var(--bg-sunken)",
        color: tone === "accent" ? "var(--accent-ink)" : "var(--fg-faint)",
      }}
    >
      {children}
    </span>
  );
}

function FilterChip({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="mono-cap"
      style={{
        padding: "var(--sp-4) var(--sp-8)",
        border: "var(--bw-hair) solid",
        borderColor: active ? "var(--accent)" : "var(--line)",
        background: active ? "var(--accent-soft)" : "var(--bg-raised)",
        borderRadius: "var(--r-1)",
        color: active ? "var(--accent-ink)" : "var(--fg-2)",
        cursor: "pointer",
        font: "inherit",
        fontSize: "var(--fs-2xs)",
      }}
    >
      {children}
    </button>
  );
}
