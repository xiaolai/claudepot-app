import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";

import { OperationProgressModal } from "../projects/OperationProgressModal";
import { LOGIN_PHASES, renderLoginResult } from "./loginProgress";

vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => {}),
}));

describe("login progress", () => {
  it("renders every login phase label in order", () => {
    render(
      <OperationProgressModal
        opId="op-login"
        title="Re-login: alice@example.com"
        phases={LOGIN_PHASES}
        fetchStatus={async () => null}
        renderResult={renderLoginResult}
        onClose={() => {}}
      />,
    );
    for (const phase of LOGIN_PHASES) {
      expect(screen.getByText(phase.label)).toBeInTheDocument();
    }
    // Internal phase ids must not leak into visible text — they live
    // in the `title` attribute for tooltips only.
    expect(screen.queryByText("spawning")).toBeNull();
    expect(screen.queryByText("waiting_for_browser")).toBeNull();
  });

  it("phases match the contract with claudepot_core::LoginPhase", () => {
    // Stable contract: this is the exact list (and order) of
    // `LoginPhase::as_str` outputs in the Rust adapter.
    expect(LOGIN_PHASES.map((p) => p.id)).toEqual([
      "spawning",
      "waiting_for_browser",
      "reading_blob",
      "fetching_profile",
      "verifying_identity",
      "persisting",
    ]);
  });
});
