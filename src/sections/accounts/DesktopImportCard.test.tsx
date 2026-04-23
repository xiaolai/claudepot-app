import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { AccountSummary, DesktopIdentity } from "../../types";

// API spy — `verifiedDesktopIdentity` is the only backend surface the
// card probes on mount; the adopt call is routed through the
// `onAdoptDesktop` prop (see Codex audit medium: "Route Desktop import
// through a shared adopt action"). Keep the mock narrow so the test
// fails loudly if a regression re-introduces a direct `api.desktopAdopt`
// call from the card.
const verifiedSpy = vi.fn();
vi.mock("../../api", () => ({
  api: {
    verifiedDesktopIdentity: (...args: unknown[]) => verifiedSpy(...args),
  },
}));

import { DesktopImportCard } from "./DesktopImportCard";

function mkAccount(overrides: Partial<AccountSummary> = {}): AccountSummary {
  return {
    uuid: "00000000-0000-0000-0000-000000000001",
    email: "alice@example.com",
    org_name: "Alice Org",
    subscription_type: "pro",
    is_cli_active: false,
    is_desktop_active: false,
    has_cli_credentials: true,
    has_desktop_profile: false,
    last_cli_switch: null,
    last_desktop_switch: null,
    token_status: "valid",
    token_remaining_mins: 120,
    credentials_healthy: true,
    verify_status: "ok",
    verified_email: "alice@example.com",
    verified_at: null,
    drift: false,
    desktop_profile_on_disk: false,
    ...overrides,
  };
}

function mkIdentity(overrides: Partial<DesktopIdentity> = {}): DesktopIdentity {
  return {
    email: "alice@example.com",
    org_uuid: "11111111-1111-1111-1111-111111111111",
    probe_method: "decrypted",
    verified_at: new Date().toISOString(),
    error: null,
    ...overrides,
  };
}

describe("DesktopImportCard", () => {
  beforeEach(() => {
    verifiedSpy.mockReset();
  });

  it("lights up Bind when the verified probe email matches a registered account", async () => {
    verifiedSpy.mockResolvedValue(mkIdentity());
    // Shared action resolves with `true` on success — the card keys
    // off this to decide whether to call `onAdopted` (modal close).
    const onAdoptDesktop = vi.fn().mockResolvedValue(true);
    const onAdopted = vi.fn();

    render(
      <DesktopImportCard
        accounts={[mkAccount()]}
        externallyDisabled={false}
        onAdoptDesktop={onAdoptDesktop}
        onAdopted={onAdopted}
      />,
    );

    // The card must call the STRICT probe, not the fast candidate path.
    await waitFor(() =>
      expect(verifiedSpy).toHaveBeenCalled(),
    );

    const bind = await screen.findByRole("button", { name: /^Bind$/ });
    expect(bind).toBeEnabled();

    await userEvent.click(bind);

    // Adopt is routed through the shared action prop — not `api.desktopAdopt`.
    await waitFor(() => expect(onAdoptDesktop).toHaveBeenCalledTimes(1));
    expect(onAdoptDesktop.mock.calls[0][0].email).toBe("alice@example.com");
    await waitFor(() =>
      expect(onAdopted).toHaveBeenCalledWith("alice@example.com"),
    );
  });

  it("refuses to render Bind when the probe returns only a candidate (not verified)", async () => {
    // Regression guard: the card MUST treat `org_uuid_candidate` as
    // unverified. Before the probe-routing fix, the card called the
    // fast sync probe (which only returns candidate tier) and every
    // user saw "Live Desktop identity could not be verified" instead
    // of Bind — this test locks that fix in place.
    verifiedSpy.mockResolvedValue(
      mkIdentity({ probe_method: "org_uuid_candidate" }),
    );

    render(
      <DesktopImportCard
        accounts={[mkAccount()]}
        externallyDisabled={false}
        onAdoptDesktop={() => Promise.resolve(true)}
        onAdopted={() => {}}
      />,
    );

    // Disabled CTAs render as non-interactive <span>s per ActionCard;
    // the Bind button only appears in the enabled "known" state.
    // Wait for the probe to resolve and the card to render "Unavailable".
    await waitFor(() =>
      expect(screen.getByText(/Unavailable/i)).toBeInTheDocument(),
    );
    expect(
      screen.queryByRole("button", { name: /^Bind$/ }),
    ).toBeNull();
  });

  it("keeps the modal open when the shared adopt action reports failure", async () => {
    verifiedSpy.mockResolvedValue(mkIdentity());
    // Regression guard for Codex audit #6: action returning `false`
    // (bind failed, toast already fired by the action) MUST NOT
    // call onAdopted — otherwise the modal closes on top of the
    // failure toast and the user thinks the bind succeeded.
    const onAdoptDesktop = vi.fn().mockResolvedValue(false);
    const onAdopted = vi.fn();

    render(
      <DesktopImportCard
        accounts={[mkAccount()]}
        externallyDisabled={false}
        onAdoptDesktop={onAdoptDesktop}
        onAdopted={onAdopted}
      />,
    );

    const bind = await screen.findByRole("button", { name: /^Bind$/ });
    await userEvent.click(bind);
    await waitFor(() => expect(onAdoptDesktop).toHaveBeenCalledTimes(1));
    expect(onAdopted).not.toHaveBeenCalled();
  });

  it("renders the Register-first state when the verified email isn't registered yet", async () => {
    verifiedSpy.mockResolvedValue(mkIdentity({ email: "nobody@example.com" }));

    render(
      <DesktopImportCard
        accounts={[mkAccount()]}
        externallyDisabled={false}
        onAdoptDesktop={() => Promise.resolve(true)}
        onAdopted={() => {}}
      />,
    );

    // Disabled CTA renders as a <span> label — check via visible text.
    await waitFor(() =>
      expect(screen.getByText(/Register first/i)).toBeInTheDocument(),
    );
    // And the Bind button must NOT be present in this state.
    expect(
      screen.queryByRole("button", { name: /^Bind$/ }),
    ).toBeNull();
  });
});
