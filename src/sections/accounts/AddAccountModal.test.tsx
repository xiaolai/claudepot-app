import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { AddAccountModal } from "./AddAccountModal";

// Mock the api module — we only exercise the three calls this modal uses.
vi.mock("../../api", () => {
  return {
    api: {
      currentCcIdentity: vi.fn(),
      accountAddFromCurrent: vi.fn(),
      accountRegisterFromBrowser: vi.fn(),
      accountLoginCancel: vi.fn(),
    },
  };
});

// Re-import after the mock is set up so test bodies can poke at the mocks.
// eslint-disable-next-line import/first
import { api } from "../../api";

const mockApi = api as unknown as {
  currentCcIdentity: ReturnType<typeof vi.fn>;
  accountAddFromCurrent: ReturnType<typeof vi.fn>;
  accountRegisterFromBrowser: ReturnType<typeof vi.fn>;
  accountLoginCancel: ReturnType<typeof vi.fn>;
};

beforeEach(() => {
  mockApi.currentCcIdentity.mockReset();
  mockApi.accountAddFromCurrent.mockReset();
  mockApi.accountRegisterFromBrowser.mockReset();
  mockApi.accountLoginCancel.mockReset();
  // Neutral preflight — no current CC session, so Import is disabled and
  // the browser login is the only active button.
  mockApi.currentCcIdentity.mockResolvedValue({ email: null, error: null });
});

describe("AddAccountModal — browser login cancel", () => {
  it("shows Cancel button while waiting for browser and invokes cancel", async () => {
    // `accountRegisterFromBrowser` never resolves in this test; we want
    // the "in-flight" UI to stay on screen while we interact with it.
    let resolveLogin!: (v: unknown) => void;
    mockApi.accountRegisterFromBrowser.mockImplementation(
      () =>
        new Promise((r) => {
          resolveLogin = r;
        }),
    );
    mockApi.accountLoginCancel.mockResolvedValue(undefined);

    render(
      <AddAccountModal
        open
        onClose={() => {}}
        onAdded={() => {}}
        onError={() => {}}
        accounts={[]}
        onAdoptDesktop={() => Promise.resolve(true)}
      />,
    );

    // Preflight resolves → the "Log in" button becomes clickable.
    const loginBtn = await screen.findByRole("button", { name: /log in/i });
    await userEvent.click(loginBtn);

    // While waiting, both Cancel affordances are visible: the inline
    // one inside the card and the prominent footer button.
    const footerCancel = await screen.findByRole("button", {
      name: /cancel login/i,
    });
    expect(footerCancel).toBeInTheDocument();
    expect(
      screen.getByLabelText("Cancel browser login"),
    ).toBeInTheDocument();

    await userEvent.click(footerCancel);
    expect(mockApi.accountLoginCancel).toHaveBeenCalledOnce();

    // Unblock the simulated register call so the component's finally
    // block runs and the Cancel UI disappears.
    resolveLogin({ email: "" });
  });

  it("swallows the cancelled-error toast when the backend reports cancellation", async () => {
    mockApi.accountRegisterFromBrowser.mockRejectedValue(
      new Error(
        "register failed: claude auth login was cancelled by the user",
      ),
    );
    const onError = vi.fn();

    render(
      <AddAccountModal
        open
        onClose={() => {}}
        onAdded={() => {}}
        onError={onError}
        accounts={[]}
        onAdoptDesktop={() => Promise.resolve(true)}
      />,
    );

    const loginBtn = await screen.findByRole("button", { name: /log in/i });
    await userEvent.click(loginBtn);

    // Let the rejected promise propagate.
    await waitFor(() =>
      expect(mockApi.accountRegisterFromBrowser).toHaveBeenCalledOnce(),
    );
    // Cancel errors are silent — no toast.
    expect(onError).not.toHaveBeenCalled();
  });

  it("cancels the backend when the modal is dismissed mid-login", async () => {
    // User clicks Log in, then tries to close the modal some other way
    // (Esc key → onClose passthrough in the real Modal component; we
    // trigger it here by calling the onClose prop directly via the
    // "Cancel login" button's sibling path — the footer still wires
    // through the same handleRequestClose).
    let resolveLogin!: (v: unknown) => void;
    mockApi.accountRegisterFromBrowser.mockImplementation(
      () =>
        new Promise((r) => {
          resolveLogin = r;
        }),
    );
    mockApi.accountLoginCancel.mockResolvedValue(undefined);
    const onClose = vi.fn();

    render(
      <AddAccountModal
        open
        onClose={onClose}
        onAdded={() => {}}
        onError={() => {}}
        accounts={[]}
        onAdoptDesktop={() => Promise.resolve(true)}
      />,
    );

    const loginBtn = await screen.findByRole("button", { name: /log in/i });
    await userEvent.click(loginBtn);

    // Simulate Esc via the modal's document-level Escape handler.
    await userEvent.keyboard("{Escape}");

    // Both the cancel command AND the parent's onClose must fire so
    // the subprocess stops AND the modal disappears.
    await waitFor(() => {
      expect(mockApi.accountLoginCancel).toHaveBeenCalledOnce();
      expect(onClose).toHaveBeenCalled();
    });

    resolveLogin({ email: "" });
  });

  it("surfaces non-cancel errors via onError", async () => {
    mockApi.accountRegisterFromBrowser.mockRejectedValue(
      new Error("register failed: claude binary not found"),
    );
    const onError = vi.fn();

    render(
      <AddAccountModal
        open
        onClose={() => {}}
        onAdded={() => {}}
        onError={onError}
        accounts={[]}
        onAdoptDesktop={() => Promise.resolve(true)}
      />,
    );

    const loginBtn = await screen.findByRole("button", { name: /log in/i });
    await userEvent.click(loginBtn);

    await waitFor(() =>
      expect(onError).toHaveBeenCalledWith(
        expect.stringMatching(/claude binary not found/),
      ),
    );
  });
});
