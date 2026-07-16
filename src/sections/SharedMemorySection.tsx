// The Knowledge section — a project-first knowledge base with a health
// dashboard, not four table-shaped lists.
//
// One section, four views arranged as a lifecycle (see
// dev-docs/knowledge-base-pane.md §3):
//   Dashboard — the landing. Health of what Claude knows. NOT usage,
//               NOT "N stored".
//   Know      — the curated base: memories + decisions + evidence,
//               project-first, with state + provenance + cross-links.
//   Review    — the triage inbox (LessonsTab). Intake: you judge, never
//               author.
//   Recall    — full-text search over raw transcripts. Explicitly NOT
//               the base.
//
// The registry id stays `shared-memory` (localStorage compat); only the
// surface changed.

import { useState } from "react";
import { LessonsTab } from "./LessonsTab";
import { KnowledgeDashboard } from "./knowledge/KnowledgeDashboard";
import { KnowView } from "./knowledge/KnowView";
import { RecallTab } from "./knowledge/RecallTab";
import { ScreenHeader } from "../shell/ScreenHeader";

type Tab = "dashboard" | "know" | "review" | "recall";
/** Which Review sub-queue a deep-link should open. */
export type QueueTarget = "proposed" | "suspect";

const TABS: { id: Tab; label: string }[] = [
  { id: "dashboard", label: "Dashboard" },
  { id: "know", label: "Know" },
  { id: "review", label: "Review" },
  { id: "recall", label: "Recall" },
];

export function SharedMemorySection() {
  const [tab, setTab] = useState<Tab>("dashboard");
  // Deep-link carriers: a Dashboard/Review jump pre-filters Know or targets
  // a Review sub-queue. `null`/default = no deep-link. These are ONE-SHOT:
  // the deep-link openers below set a carrier and jump with the raw setter,
  // but `navTab` (a plain tab click / arrow key) clears them — otherwise a
  // project drilled into once would keep silently filtering the Know tab for
  // the rest of the session, making the base look empty or broken (an
  // invisible filter is the most corrosive failure a knowledge base can have).
  const [knowProject, setKnowProject] = useState<string | null>(null);
  const [knowMemoryId, setKnowMemoryId] = useState<string | null>(null);
  const [reviewQueue, setReviewQueue] = useState<QueueTarget>("proposed");

  const openProjectInKnow = (projectPath: string) => {
    setKnowProject(projectPath);
    setKnowMemoryId(null);
    setTab("know");
  };

  const openMemoryInKnow = (projectPath: string, memoryId: string) => {
    setKnowProject(projectPath);
    setKnowMemoryId(memoryId);
    setTab("know");
  };

  const openReview = (queue: QueueTarget = "proposed") => {
    setReviewQueue(queue);
    setTab("review");
  };

  // A direct tab navigation is not a deep-link, so it drops any stale
  // carrier before switching. Deep-link openers above bypass this by calling
  // `setTab` straight, so their carrier survives the jump.
  const navTab = (next: Tab) => {
    if (next === "know") {
      setKnowProject(null);
      setKnowMemoryId(null);
    }
    if (next === "review") setReviewQueue("proposed");
    setTab(next);
  };

  // WAI-ARIA tabs pattern: Left/Right move selection with wrap-around and
  // focus follows. Companion to TabButton's roving tabIndex — without it,
  // inactive tabs (tabIndex={-1}) would be keyboard-unreachable.
  const onTablistKeyDown = (e: React.KeyboardEvent<HTMLElement>) => {
    if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
    e.preventDefault();
    const delta = e.key === "ArrowRight" ? 1 : -1;
    const i = TABS.findIndex((t) => t.id === tab);
    const next = TABS[(i + delta + TABS.length) % TABS.length]!.id;
    navTab(next);
    document.getElementById(`shared-memory-tab-${next}`)?.focus();
  };

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        minHeight: 0,
      }}
    >
      <ScreenHeader
        title="Knowledge"
        subtitle="What Claude knows — health, curated base, triage, and recall"
      />
      <nav
        role="tablist"
        aria-label="Knowledge tabs"
        onKeyDown={onTablistKeyDown}
        style={{
          display: "flex",
          gap: "var(--sp-16)",
          padding: "0 var(--sp-24)",
          borderBottom: "var(--sp-px) solid var(--line)",
        }}
      >
        {TABS.map((t) => (
          <TabButton
            key={t.id}
            id={`shared-memory-tab-${t.id}`}
            panelId={`shared-memory-panel-${t.id}`}
            active={tab === t.id}
            onClick={() => navTab(t.id)}
          >
            {t.label}
          </TabButton>
        ))}
      </nav>
      <div
        role="tabpanel"
        id={`shared-memory-panel-${tab}`}
        aria-labelledby={`shared-memory-tab-${tab}`}
        style={{
          flex: 1,
          minHeight: 0,
          overflow: "auto",
          padding: "var(--sp-24)",
        }}
      >
        {tab === "dashboard" && (
          <KnowledgeDashboard
            onOpenProject={openProjectInKnow}
            onOpenReview={openReview}
          />
        )}
        {tab === "know" && (
          <KnowView
            initialProjectFilter={knowProject}
            initialMemoryId={knowMemoryId}
            onReview={openReview}
          />
        )}
        {tab === "review" && (
          <LessonsTab initialQueue={reviewQueue} onOpenMemory={openMemoryInKnow} />
        )}
        {tab === "recall" && <RecallTab />}
      </div>
    </div>
  );
}

// Mirrors the canonical SectionTab ARIA contract
// (src/sections/sessions/components/SectionTab.tsx): id + aria-controls
// wired to the tabpanel, roving tabIndex (the tablist above supplies the
// arrow-key navigation that keeps inactive tabs reachable), and the design
// system's `pm-focus` ring. Only the visuals stay on this section's
// underline style.
function TabButton({
  id,
  panelId,
  active,
  onClick,
  children,
}: {
  id: string;
  panelId: string;
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      id={id}
      type="button"
      role="tab"
      aria-selected={active}
      aria-controls={panelId}
      tabIndex={active ? 0 : -1}
      className="pm-focus"
      onClick={onClick}
      style={{
        padding: "var(--sp-12) 0",
        marginBottom: "calc(-1 * var(--sp-px))",
        borderBottom: active
          ? "var(--sp-2) solid var(--accent)"
          : "var(--sp-2) solid transparent",
        background: "transparent",
        color: active ? "var(--fg)" : "var(--fg-muted)",
        font: "inherit",
        fontWeight: active ? 600 : 400,
        cursor: "pointer",
      }}
    >
      {children}
    </button>
  );
}
