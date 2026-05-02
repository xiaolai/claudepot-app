import { useEffect, useMemo, useRef, useState } from "react";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import { Modal } from "../../components/primitives/Modal";
import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";
import type {
  TemplateCategory,
  TemplateSummaryDto,
} from "../../types";
import { TemplateInstallView } from "./TemplateInstallView";

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
 * Single Modal that swaps between the gallery view and the
 * install view based on internal state. Eliminates the
 * close-modal-then-open-modal flash that two separate `<Modal>`
 * elements produced when transitioning gallery → install.
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

  // Parent callsites pass inline lambdas, so onError changes
  // identity on every parent render. Keep the latest in a ref so
  // the fetch effect only fires on `open` transitions — otherwise
  // every parent re-render (toast, runsRefreshKey, etc.) reset
  // installTarget to null and snapped the install view back to
  // the gallery mid-transition.
  const onErrorRef = useRef(onError);
  useEffect(() => {
    onErrorRef.current = onError;
  }, [onError]);

  useEffect(() => {
    if (!open) return;
    setTemplates(null);
    setInstallTarget(null);
    api
      .templatesList()
      .then(setTemplates)
      .catch((e: unknown) => onErrorRef.current(String(e)));
  }, [open]);

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

  return (
    <Modal
      open={open}
      onClose={onClose}
      width="lg"
      aria-labelledby="template-gallery-title"
    >
      {installTarget !== null ? (
        <TemplateInstallView
          templateId={installTarget}
          onBack={() => setInstallTarget(null)}
          onInstalled={() => {
            setInstallTarget(null);
            onInstalled();
            onClose();
          }}
          onError={onError}
          onOpenThirdParties={onOpenThirdParties}
          backLabel="Back"
        />
      ) : (
        <GalleryGrid
          templates={templates}
          visible={visible}
          categories={categories}
          filter={filter}
          onFilter={setFilter}
          onPick={setInstallTarget}
          onClose={onClose}
        />
      )}
    </Modal>
  );
}

function GalleryGrid({
  templates,
  visible,
  categories,
  filter,
  onFilter,
  onPick,
  onClose,
}: {
  templates: TemplateSummaryDto[] | null;
  visible: TemplateSummaryDto[];
  categories: TemplateCategory[];
  filter: TemplateCategory | "all";
  onFilter: (next: TemplateCategory | "all") => void;
  onPick: (id: string) => void;
  onClose: () => void;
}) {
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        // Pin both gallery and install view to the same vertical
        // extent so swapping content inside the same Modal does
        // not resize the dialog box (the resize was visible as a
        // flash). No `flex: 1` here — in this flex-column parent
        // the `flex: 1 1 0%` basis collapses the wrapper to its
        // non-flex children's intrinsic height, which would
        // override the explicit `height` and leave the body
        // body shrunk to ~0. The `height` alone is sufficient.
        height: "var(--modal-body-cap-md)",
        width: "100%",
        minHeight: 0,
      }}
    >
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "var(--sp-12)",
          padding: "var(--sp-16) var(--sp-20) var(--sp-12)",
          borderBottom: "var(--bw-hair) solid var(--line)",
          flexShrink: 0,
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
            <FilterChip active={filter === "all"} onClick={() => onFilter("all")}>
              All
            </FilterChip>
            {categories.map((c) => (
              <FilterChip
                key={c}
                active={filter === c}
                onClick={() => onFilter(c)}
              >
                {CATEGORY_LABELS[c]}
              </FilterChip>
            ))}
          </div>
        )}
      </div>

      {/* Scrollable cards body */}
      <div
        style={{
          padding: "var(--sp-16) var(--sp-20)",
          overflowY: "auto",
          flex: 1,
          minHeight: 0,
        }}
      >
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
                onClick={() => onPick(t.id)}
              />
            ))}
          </div>
        )}
      </div>

      {/* Bottom spacer — also a sticky-ish footer-area buffer
          so the last row of cards doesn't kiss the modal edge. */}
      <div
        style={{
          flexShrink: 0,
          height: "var(--sp-12)",
        }}
      />
    </div>
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
      <div style={{ display: "flex", alignItems: "center", gap: "var(--sp-8)" }}>
        <Glyph g={NF.dot} />
        <span style={{ fontSize: "var(--fs-md)", color: "var(--fg)" }}>
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
        className="mono-cap"
        style={{
          display: "flex",
          flexWrap: "wrap",
          gap: "var(--sp-6)",
          marginTop: "auto",
          paddingTop: "var(--sp-6)",
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-faint)",
        }}
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
