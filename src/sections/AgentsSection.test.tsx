import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { AgentSummaryDto } from "../types";

const h = vi.hoisted(() => ({
  agentsList: vi.fn(),
  routesList: vi.fn(),
  agentsSchedulerCapabilities: vi.fn(),
  agentsRunNowStart: vi.fn(),
  listen: vi.fn(),
}));

vi.mock("../api", () => ({
  api: {
    agentsList: h.agentsList,
    routesList: h.routesList,
    agentsSchedulerCapabilities: h.agentsSchedulerCapabilities,
    agentsRunNowStart: h.agentsRunNowStart,
  },
}));
vi.mock("../providers/AppStateProvider", () => ({
  useAppState: () => ({ pushToast: vi.fn() }),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: h.listen,
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
// Stub the card down to the one affordance the run-lifecycle test
// needs: a button that fires `onRun(agent.id)`.
vi.mock("./agents/AgentCard", () => ({
  AgentCard: ({
    agent,
    onRun,
  }: {
    agent: AgentSummaryDto;
    onRun: (id: string) => void;
  }) => (
    <button type="button" onClick={() => onRun(agent.id)}>
      run-agent-stub
    </button>
  ),
}));

import { AgentsSection } from "./AgentsSection";
import { sectionIds } from "./registry";

function agent(partial: Partial<AgentSummaryDto> = {}): AgentSummaryDto {
  return {
    id: "a1",
    name: "nightly-audit",
    display_name: null,
    description: null,
    enabled: true,
    binary_kind: "first_party",
    binary_route_id: null,
    model: null,
    cwd: "/tmp/proj",
    permission_mode: "default",
    allowed_tools: [],
    max_budget_usd: null,
    trigger_kind: "manual",
    cron: null,
    timezone: null,
    event_kind: null,
    event_debounce_secs: null,
    lifecycle: "installed",
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    ...partial,
  };
}

beforeEach(() => {
  h.agentsList.mockReset().mockResolvedValue([]);
  h.routesList.mockReset().mockResolvedValue([]);
  h.agentsSchedulerCapabilities
    .mockReset()
    .mockResolvedValue({ scheduler: "launchd", available: true });
  h.agentsRunNowStart.mockReset();
  h.listen.mockReset().mockResolvedValue(() => {});
});

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

describe("AgentsSection — run-now listener lifecycle", () => {
  it("drains the op-progress listener when unmounted mid-run", async () => {
    // Regression (audit 2026-07 F5): handleRun's unlisten + safety
    // timeout were handler-scoped only, so unmounting mid-run leaked
    // them and late op events called setState on an unmounted
    // component.
    const unlisten = vi.fn();
    h.agentsList.mockResolvedValue([agent()]);
    h.agentsRunNowStart.mockResolvedValue("op-1");
    h.listen.mockResolvedValue(unlisten);

    const { unmount } = render(<AgentsSection />);
    await userEvent.click(
      await screen.findByRole("button", { name: "run-agent-stub" }),
    );
    await waitFor(() =>
      expect(h.listen).toHaveBeenCalledWith(
        "op-progress::op-1",
        expect.any(Function),
      ),
    );
    // Flush the microtask that assigns the resolved unlisten.
    await act(async () => {});

    unmount();
    expect(unlisten).toHaveBeenCalled();
  });
});
