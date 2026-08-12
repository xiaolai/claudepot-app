import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";

import type { ProjectInfo, TargetProbe } from "../../types";
import { OperationsProvider } from "../../hooks/useOperations";

const moveStartSpy = vi.fn();
const moveStatusSpy = vi.fn();
const probeSpy = vi.fn();
vi.mock("../../api", () => ({
  api: {
    sessionMoveStart: (...args: unknown[]) => moveStartSpy(...args),
    sessionMoveStatus: (...args: unknown[]) => moveStatusSpy(...args),
    sessionMoveProbeTarget: (...args: unknown[]) => probeSpy(...args),
  },
}));
const openDialogSpy = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: (...args: unknown[]) => openDialogSpy(...args),
}));

import { MoveSessionModal } from "./MoveSessionModal";

function mkProject(overrides: Partial<ProjectInfo> = {}): ProjectInfo {
  return {
    sanitized_name: "-live",
    original_path: "/live",
    session_count: 0,
    memory_file_count: 0,
    total_size_bytes: 0,
    last_modified_ms: null,
    is_orphan: false,
    is_reachable: true,
    is_empty: false,
    ...overrides,
  };
}

function mkProbe(overrides: Partial<TargetProbe> = {}): TargetProbe {
  return {
    resolvedPath: "/new/place",
    isAbsolute: true,
    exists: false,
    isDir: false,
    ...overrides,
  };
}

const baseProps = {
  sessionId: "abcd0000-0000-0000-0000-000000000000",
  fromCwd: "/from",
  projects: [
    mkProject({ original_path: "/from", sanitized_name: "-from" }),
    mkProject({ original_path: "/live/main", sanitized_name: "-live-main" }),
    mkProject({ original_path: "/live/other", sanitized_name: "-live-other" }),
  ],
};

function withProvider(ui: ReactNode) {
  return <OperationsProvider>{ui}</OperationsProvider>;
}

function renderModal(props: Partial<Parameters<typeof MoveSessionModal>[0]> = {}) {
  return render(
    withProvider(
      <MoveSessionModal
        {...baseProps}
        onClose={() => {}}
        onCompleted={() => {}}
        {...props}
      />,
    ),
  );
}

/** Accessible names of the rendered listbox rows, in visible order. */
function optionNames(): string[] {
  return screen.getAllByRole("option").map((o) => o.textContent ?? "");
}

describe("MoveSessionModal", () => {
  beforeEach(() => {
    moveStartSpy.mockReset();
    moveStatusSpy.mockReset();
    openDialogSpy.mockReset();
    probeSpy.mockReset();
  });

  it("excludes the source cwd from the target list", () => {
    renderModal();
    const names = optionNames().join("|");
    expect(names).not.toContain("/from");
    expect(names).toContain("/live/main");
  });

  it("excludes orphan / unreachable / empty projects from targets (B1)", () => {
    renderModal({
      projects: [
        mkProject({ original_path: "/from", sanitized_name: "-from" }),
        mkProject({ original_path: "/live/ok", sanitized_name: "-live-ok" }),
        mkProject({
          original_path: "/live/dead",
          sanitized_name: "-live-dead",
          is_orphan: true,
        }),
        mkProject({
          original_path: "/live/offline",
          sanitized_name: "-live-offline",
          is_reachable: false,
        }),
        mkProject({
          original_path: "/live/empty",
          sanitized_name: "-live-empty",
          is_empty: true,
        }),
      ],
    });
    const names = optionNames().join("|");
    expect(names).toContain("/live/ok");
    expect(names).not.toContain("/live/dead");
    expect(names).not.toContain("/live/offline");
    expect(names).not.toContain("/live/empty");
  });

  it("defaults to the most-recently-touched alive project (B11)", () => {
    renderModal({
      projects: [
        mkProject({ original_path: "/from", sanitized_name: "-from" }),
        mkProject({
          original_path: "/old",
          sanitized_name: "-old",
          last_modified_ms: 1_000,
        }),
        mkProject({
          original_path: "/fresh",
          sanitized_name: "-fresh",
          last_modified_ms: 9_999_999_999,
        }),
        mkProject({
          original_path: "/mid",
          sanitized_name: "-mid",
          last_modified_ms: 5_000,
        }),
      ],
    });
    const selected = screen
      .getAllByRole("option")
      .filter((o) => o.getAttribute("aria-selected") === "true");
    expect(selected).toHaveLength(1);
    expect(selected[0].textContent).toContain("/fresh");
    expect(
      screen.getByRole("button", { name: /Move to fresh/i }),
    ).toBeEnabled();
  });

  it("filters the list as you type", async () => {
    const user = userEvent.setup();
    renderModal();
    expect(optionNames()).toHaveLength(2);
    await user.type(screen.getByRole("combobox"), "other");
    await waitFor(() => expect(optionNames()).toHaveLength(1));
    expect(optionNames()[0]).toContain("/live/other");
  });

  it("arrow keys move the cursor and Enter picks that row", async () => {
    const user = userEvent.setup();
    renderModal();
    const input = screen.getByRole("combobox");
    input.focus();
    await user.keyboard("{ArrowDown}{Enter}");
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: /Move to other/i }),
      ).toBeEnabled(),
    );
  });

  it("passes the picked project through with createTargetDir false", async () => {
    moveStartSpy.mockResolvedValue("op-handoff");
    const onClose = vi.fn();
    const user = userEvent.setup();
    renderModal({ onClose });

    await user.click(screen.getByRole("option", { name: /\/live\/other/ }));
    await user.click(screen.getByRole("button", { name: /Move to/i }));

    await waitFor(() =>
      expect(moveStartSpy).toHaveBeenCalledWith({
        sessionId: baseProps.sessionId,
        fromCwd: "/from",
        toCwd: "/live/other",
        forceLive: false,
        forceConflict: false,
        cleanupSource: false,
        createTargetDir: false,
      }),
    );
    // The local modal closes once the shell takes over.
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  // ── The folder-that-doesn't-exist-yet path ────────────────────────

  it("offers a typed absolute path as a target and says it will be created", async () => {
    probeSpy.mockResolvedValue(
      mkProbe({ resolvedPath: "/brand/new", exists: false }),
    );
    const user = userEvent.setup();
    renderModal();

    await user.type(screen.getByRole("combobox"), "/brand/new");
    await user.click(screen.getByRole("option", { name: /Use this folder/ }));

    await waitFor(() =>
      expect(screen.getByText(/doesn't exist yet/i)).toBeInTheDocument(),
    );
    expect(probeSpy).toHaveBeenCalledWith("/brand/new");
    expect(
      screen.getByRole("button", { name: /Create & move to new/i }),
    ).toBeEnabled();
  });

  it("submits the resolved path with createTargetDir true", async () => {
    // Probe expands `~`; the move must receive the expanded path, not
    // the literal the user typed.
    probeSpy.mockResolvedValue(
      mkProbe({ resolvedPath: "/Users/me/brand/new", exists: false }),
    );
    moveStartSpy.mockResolvedValue("op-create");
    const user = userEvent.setup();
    renderModal();

    await user.type(screen.getByRole("combobox"), "~/brand/new");
    await user.click(screen.getByRole("option", { name: /Use this folder/ }));
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: /Create & move/i }),
      ).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: /Create & move/i }));

    await waitFor(() =>
      expect(moveStartSpy).toHaveBeenCalledWith(
        expect.objectContaining({
          toCwd: "/Users/me/brand/new",
          createTargetDir: true,
        }),
      ),
    );
  });

  it("an existing folder submits without asking to create it", async () => {
    probeSpy.mockResolvedValue(
      mkProbe({ resolvedPath: "/already/there", exists: true, isDir: true }),
    );
    const user = userEvent.setup();
    renderModal();

    await user.type(screen.getByRole("combobox"), "/already/there");
    await user.click(screen.getByRole("option", { name: /Use this folder/ }));

    // Both assertions live inside the wait. The badge and the button
    // are separate state updates, so waiting only for the badge and
    // then asserting the button synchronously races whichever render
    // lands second — that is the intermittent failure CI saw on
    // 2026-08-12 (`Unable to find … /Move to there/i`), not a timeout.
    await waitFor(() => {
      expect(screen.getByText(/Existing folder/i)).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: /Move to there/i }),
      ).toBeEnabled();
    });
  });

  it("refuses a path that is a file, stating the reason inline", async () => {
    probeSpy.mockResolvedValue(
      mkProbe({ resolvedPath: "/notes.txt", exists: true, isDir: false }),
    );
    const user = userEvent.setup();
    renderModal();

    await user.type(screen.getByRole("combobox"), "/notes.txt");
    await user.click(screen.getByRole("option", { name: /Use this folder/ }));

    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent(
        /file, not a folder/i,
      ),
    );
    expect(screen.getByRole("button", { name: /Move to/i })).toBeDisabled();
  });

  it("refuses a relative path", async () => {
    probeSpy.mockResolvedValue(
      mkProbe({ resolvedPath: "~someone/x", isAbsolute: false }),
    );
    const user = userEvent.setup();
    renderModal();

    await user.type(screen.getByRole("combobox"), "~someone/x");
    await user.click(screen.getByRole("option", { name: /Use this folder/ }));

    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent(/absolute path/i),
    );
    expect(screen.getByRole("button", { name: /Move to/i })).toBeDisabled();
  });

  it("does not offer to create a folder for a plain filter term", async () => {
    const user = userEvent.setup();
    renderModal();
    await user.type(screen.getByRole("combobox"), "main");
    expect(screen.queryByText(/Use this folder/)).not.toBeInTheDocument();
    expect(probeSpy).not.toHaveBeenCalled();
  });

  it("Browse picks a directory and probes it", async () => {
    openDialogSpy.mockResolvedValue("/picked");
    probeSpy.mockResolvedValue(
      mkProbe({ resolvedPath: "/picked", exists: true, isDir: true }),
    );
    const user = userEvent.setup();
    renderModal();

    await user.click(screen.getByRole("button", { name: /Browse/i }));
    // Wait on the rendered consequence, not on the spy: `browse` awaits
    // the dialog and then the probe, so "the probe was called" is true a
    // render before the target actually changes.
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: /Move to picked/i }),
      ).toBeEnabled(),
    );
    expect(probeSpy).toHaveBeenCalledWith("/picked");
  });

  it("disables Move when the target equals the source", async () => {
    probeSpy.mockResolvedValue(
      mkProbe({ resolvedPath: "/from", exists: true, isDir: true }),
    );
    const user = userEvent.setup();
    renderModal();

    await user.type(screen.getByRole("combobox"), "/from");
    await user.click(screen.getByRole("option", { name: /Use this folder/ }));

    await waitFor(() =>
      expect(screen.getByText(/Existing folder/i)).toBeInTheDocument(),
    );
    expect(screen.getByRole("button", { name: /Move to/i })).toBeDisabled();
  });

  // ── Everything downstream of the target ───────────────────────────

  it("threads cleanupSource from the Advanced toggle (B6)", async () => {
    moveStartSpy.mockResolvedValue("op-test-1");
    const user = userEvent.setup();
    renderModal();
    await user.click(screen.getByText("Advanced"));
    await user.click(
      screen.getByLabelText(/remove source project dir if it's empty/i),
    );
    await user.click(screen.getByRole("button", { name: /Move to/i }));

    await waitFor(() =>
      expect(moveStartSpy).toHaveBeenCalledWith(
        expect.objectContaining({ cleanupSource: true }),
      ),
    );
  });

  it("threads forceLive / forceConflict into the api call", async () => {
    moveStartSpy.mockResolvedValue("op-flags");
    const user = userEvent.setup();
    renderModal();
    await user.click(screen.getByText("Advanced"));
    await user.click(
      screen.getByLabelText(/force past the live-session mtime guard/i),
    );
    await user.click(screen.getByRole("button", { name: /Move to/i }));

    await waitFor(() =>
      expect(moveStartSpy).toHaveBeenCalledWith(
        expect.objectContaining({ forceLive: true, forceConflict: false }),
      ),
    );
  });

  it("shows inline error when the start call rejects, without closing", async () => {
    moveStartSpy.mockRejectedValue("session appears live (mtime < threshold)");
    const onClose = vi.fn();
    const user = userEvent.setup();
    renderModal({ onClose });
    await user.click(screen.getByRole("button", { name: /Move to/i }));

    await waitFor(() =>
      expect(screen.getByText(/appears live/)).toBeInTheDocument(),
    );
    // The local modal stays open so the user can fix the inputs.
    expect(onClose).not.toHaveBeenCalled();
  });

  it("closes on Escape", () => {
    const onClose = vi.fn();
    renderModal({ onClose });
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });
});
