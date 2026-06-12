import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

vi.mock("../api", () => ({
  api: {
    agentsList: vi.fn(() => Promise.resolve([])),
    routesList: vi.fn(() => Promise.resolve([])),
    agentsSchedulerCapabilities: vi.fn(() =>
      Promise.resolve({ scheduler: "launchd", available: true }),
    ),
  },
}));
vi.mock("../providers/AppStateProvider", () => ({
  useAppState: () => ({ pushToast: vi.fn() }),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => {}),
}));
// Stub the gallery: render the open-Providers affordance directly so
// the test exercises AgentsSection's handler without the full modal.
vi.mock("./templates/TemplateGallery", () => ({
  TemplateGallery: ({
    onOpenThirdParties,
  }: {
    onOpenThirdParties: () => void;
  }) => (
    <button type="button" onClick={onOpenThirdParties}>
      open-providers-stub
    </button>
  ),
}));
vi.mock("./agents/AgentModals", () => ({
  AddAgentModal: () => null,
  AddFromBuiltinTemplateModal: () => null,
  EditAgentModal: () => null,
  ReviewInstallModal: () => null,
}));
vi.mock("./agents/AgentCard", () => ({ AgentCard: () => null }));

import { AgentsSection } from "./AgentsSection";
import { sectionIds } from "./registry";

describe("AgentsSection — Providers deep link", () => {
  it("dispatches claudepot:navigate-section with a registry-valid id", async () => {
    // The App shell's only nav listener is `claudepot:navigate-section`
    // reading `detail.id` against the registry ids. A previous
    // regression dispatched a dead channel ("claudepot:nav") with a
    // wrong key ("section") and a wrong id ("third-parties").
    const seen: Array<{ id?: string }> = [];
    const handler = (e: Event) => {
      seen.push((e as CustomEvent<{ id?: string }>).detail ?? {});
    };
    window.addEventListener("claudepot:navigate-section", handler);
    try {
      render(<AgentsSection />);
      await userEvent.click(
        await screen.findByRole("button", { name: "open-providers-stub" }),
      );
      expect(seen).toHaveLength(1);
      expect(seen[0].id).toBe("third-party");
      // Pin the cross-file contract: the dispatched id must exist in
      // the nav registry, so a registry rename breaks this test.
      expect(sectionIds).toContain(seen[0].id);
    } finally {
      window.removeEventListener("claudepot:navigate-section", handler);
    }
  });
});
