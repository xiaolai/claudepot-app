import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import type { AccountSummary } from "../../types";
import { AnomalyBanner, isAnomaly } from "./AnomalyBanner";
import { HealthChips } from "./HealthChips";
import { HealthFooter } from "./HealthFooter";
import { requiresRelogin, verifyKind } from "./verifyStatus";

/**
 * The user-visible half of issue #74.
 *
 * Claude Code can sign *itself* out by overwriting its keychain item
 * with a cleared-credentials sentinel instead of deleting it. Core now
 * reports that as the terminal `signed_out` status; these tests cover
 * what the account card does with it.
 *
 * The failure being locked out is not "wrong wording" — it is a
 * terminal state rendered as a transient one. An account that only a
 * re-login can recover showed a grey "unverified" chip and no banner,
 * so the surface offered the user nothing to click and implied waiting
 * would fix it.
 */
function mkAccount(overrides: Partial<AccountSummary> = {}): AccountSummary {
  return {
    uuid: "acct-1",
    email: "a@example.com",
    org_name: "personal",
    subscription_type: null,
    is_cli_active: true,
    is_desktop_active: false,
    has_cli_credentials: true,
    credentials_healthy: true,
    has_desktop_profile: false,
    desktop_profile_on_disk: false,
    verify_status: "signed_out",
    verified_email: "a@example.com",
    verified_at: "2026-08-14T09:59:00Z",
    drift: false,
    // The sentinel zeroes `expiresAt`, so the card's token cell reads
    // "expired" — which is precisely why the verify status has to carry
    // the real signal. This fixture keeps that misleading value.
    token_status: "expired",
    token_remaining_mins: -1,
    last_cli_switch: null,
    last_desktop_switch: null,
    ...overrides,
  };
}

describe("signed-out accounts are actionable, not 'unverified'", () => {
  it("counts as broken, not as unverified", () => {
    render(<HealthChips accounts={[mkAccount()]} />);
    // The chips are icon+count pairs keyed by aria-label.
    expect(screen.getByLabelText("1 broken")).toBeInTheDocument();
    expect(screen.queryByLabelText("1 unverified")).toBeNull();
  });

  it("does not dilute the healthy count", () => {
    render(
      <HealthChips
        accounts={[
          mkAccount(),
          mkAccount({ uuid: "acct-2", verify_status: "ok" }),
        ]}
      />,
    );
    expect(screen.getByLabelText("1 verified")).toBeInTheDocument();
    expect(screen.getByLabelText("1 broken")).toBeInTheDocument();
  });

  it("is an anomaly, so the card renders the banner that carries Re-login", () => {
    expect(isAnomaly(mkAccount())).toBe(true);
  });

  it("offers Re-login and blames neither the server nor the account", () => {
    render(<AnomalyBanner account={mkAccount()} />);

    expect(screen.getByRole("alert")).toBeInTheDocument();
    expect(
      screen.getByText("Claude Code signed itself out"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Re-login" }),
    ).toBeInTheDocument();

    // The copy must not reuse the `rejected` story. Nothing refused
    // this login — with empty tokens no request was ever sent — and
    // saying otherwise sends the user to check an account that is fine.
    const alert = screen.getByRole("alert");
    expect(alert.textContent).not.toMatch(/reject/i);
    expect(alert.textContent).toMatch(/account is fine/i);
  });

  /// The counterpart guard: a transient failure must NOT grow a banner.
  /// If it did, every network blip would nag the user to re-login —
  /// which is the mirror-image defect and just as wrong.
  it("leaves a transient network_error alone", () => {
    const transient = mkAccount({ verify_status: "network_error" });
    expect(isAnomaly(transient)).toBe(false);
    const { container } = render(<AnomalyBanner account={transient} />);
    expect(container).toBeEmptyDOMElement();

    render(<HealthChips accounts={[transient]} />);
    expect(screen.getByLabelText("1 unverified")).toBeInTheDocument();
  });
});

/**
 * The shared classifier every account surface now derives from.
 *
 * Before it existed, four surfaces answered "what does this status
 * mean?" independently and `signed_out` had to be added to each. These
 * tests pin the mapping in one place so a fifth surface cannot quietly
 * disagree.
 */
describe("verifyKind", () => {
  it("groups both terminal statuses under one kind", () => {
    expect(verifyKind("rejected")).toBe("needsLogin");
    expect(verifyKind("signed_out")).toBe("needsLogin");
    expect(requiresRelogin("signed_out")).toBe(true);
    expect(requiresRelogin("rejected")).toBe(true);
  });

  it("keeps drift separate — verify can clear it, so it is not a re-login", () => {
    expect(verifyKind("drift")).toBe("drift");
    expect(requiresRelogin("drift")).toBe(false);
  });

  it("treats not-yet-checked and could-not-check alike", () => {
    expect(verifyKind("never")).toBe("unknown");
    expect(verifyKind("network_error")).toBe("unknown");
    expect(requiresRelogin("network_error")).toBe(false);
  });

  it("reports ok only for ok", () => {
    expect(verifyKind("ok")).toBe("ok");
  });

  /// The runtime half of the exhaustiveness guard. A status from a
  /// newer backend must degrade to "keep checking", never to "ok" —
  /// claiming health we have not established is the failure mode this
  /// whole change exists to remove.
  it("degrades an unknown status to 'unknown', never to 'ok'", () => {
    const future = "some_future_status" as Parameters<typeof verifyKind>[0];
    expect(verifyKind(future)).toBe("unknown");
  });
});

describe("HealthFooter — status labels are exhaustive", () => {
  it("labels every status distinctly instead of falling through", () => {
    const labels = new Map<string, string>();
    for (const status of [
      "never",
      "ok",
      "drift",
      "rejected",
      "signed_out",
      "network_error",
    ] as const) {
      const { container, unmount } = render(
        <HealthFooter account={mkAccount({ verify_status: status })} />,
      );
      labels.set(status, container.textContent ?? "");
      unmount();
    }
    // The failure this guards: a status with no branch rendering the
    // same "not yet verified" text as `never`.
    expect(labels.get("signed_out")).not.toBe(labels.get("never"));
    expect(labels.get("signed_out")).not.toBe(labels.get("rejected"));
    expect(labels.get("signed_out")).not.toBe(labels.get("network_error"));
  });
});
