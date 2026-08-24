import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { RemoteStatus } from "../../api/remote";
import { i18n } from "../../lib/i18n";
import SOURCE from "./RemotePane.tsx?raw";

const remoteStatusMock = vi.fn();
const remoteStartMock = vi.fn();
const remoteStopMock = vi.fn();
const remoteEnableMock = vi.fn();
const remoteDisableMock = vi.fn();
const remoteSetPasswordMock = vi.fn();
const remoteSetApprovalsMock = vi.fn();
const remoteRevokeAllMock = vi.fn();
const remoteRevokeDeviceMock = vi.fn();

// `quickPromptApi` is mocked because the pane now renders the
// quick-prompt editor as a section — see `RemotePane`'s module docs on
// why those chips belong here rather than in a pane of their own. Its
// own behaviour is covered by its own component; here it just has to
// mount without reaching for an IPC bridge that is not there.
vi.mock("../../api", () => ({
  quickPromptApi: {
    list: () => Promise.resolve([]),
    save: (p: unknown) => Promise.resolve(p),
    defaults: () => Promise.resolve([]),
  },
  remoteApi: {
    remoteStatus: (...a: unknown[]) => remoteStatusMock(...a),
    remoteStart: (...a: unknown[]) => remoteStartMock(...a),
    remoteStop: (...a: unknown[]) => remoteStopMock(...a),
    remoteEnable: (...a: unknown[]) => remoteEnableMock(...a),
    remoteDisable: (...a: unknown[]) => remoteDisableMock(...a),
    remoteSetPassword: (...a: unknown[]) => remoteSetPasswordMock(...a),
    remoteSetApprovals: (...a: unknown[]) => remoteSetApprovalsMock(...a),
    remoteRevokeAll: (...a: unknown[]) => remoteRevokeAllMock(...a),
    remoteRevokeDevice: (...a: unknown[]) => remoteRevokeDeviceMock(...a),
  },
}));

import { RemotePane } from "./RemotePane";

function status(over: Partial<RemoteStatus> = {}): RemoteStatus {
  return {
    enabled: true,
    serving: false,
    runningHere: false,
    url: null,
    bind: "127.0.0.1",
    port: 8420,
    exposure: "loopback",
    bindError: null,
    requiresTls: false,
    passwordSet: true,
    totpEnabled: false,
    passkeys: 0,
    approvalsEnabled: true,
    configRecovered: false,
    devicesRecovered: false,
    lastError: null,
    warnings: [],
    devices: [],
    activeDevices: 0,
    ...over,
  };
}

/**
 * The English string a key resolves to.
 *
 * `i18n.t` is typed against the literal catalog keys, which is right in
 * the app and wrong for a helper that takes one as a parameter. The
 * cast is confined here rather than repeated at 40 call sites — and it
 * is safe in the direction that matters: a key that does not exist
 * resolves to itself, so the assertion fails on a string nobody
 * rendered rather than passing silently.
 */
const t = (k: string) => i18n.t(k as never, { ns: "settings" }) as string;

beforeEach(async () => {
  vi.clearAllMocks();
  await i18n.changeLanguage("en");
});

afterEach(() => {
  vi.useRealTimers();
});

/**
 * The eight class names this pane once invented.
 *
 * It shipped rendering `pane`, `pane-block`, `pane-intro`,
 * `pane-warning`, `pane-error`, `pane-actions`, `remote-devices` and
 * `status-chip` — and only `remote-devices` was ever given a rule. A
 * className with no rule is valid HTML, invisible to `tsc`, and
 * invisible to every test in this file, all of which assert on text. So
 * the pane rendered as unstyled markup with a full green suite.
 *
 * The general check is `scripts/check-classes.mjs`, which scans every
 * `className` in `src/` against every stylesheet and runs in CI — it
 * cannot live here, because this file is typechecked by the browser
 * tsconfig (no node types) and Vite stubs `?raw` on CSS under Vitest.
 * What stays is the file-local lock: these particular names, by name,
 * so the specific mistake cannot come back quietly.
 */
describe("RemotePane — the design system is actually wired", () => {
  it("uses none of the class names the first version invented", () => {
    for (const dead of [
      "pane-block",
      "pane-intro",
      "pane-warning",
      "pane-error",
      "pane-actions",
      "status-chip",
    ]) {
      expect(SOURCE).not.toContain(dead);
    }
  });

  it("renders through the primitives rather than hand-rolled markup", () => {
    // `SectionLabel` for headings, `Input` for fields, `Button` for
    // actions. The first version used bare `<h2>`, `<input>` and its own
    // classes — and the `<h2>` duplicated the title `SettingsSection`
    // already renders from the pane registry.
    expect(SOURCE).toContain("SectionLabel");
    expect(SOURCE).toContain("<Input");
    expect(SOURCE).not.toContain("<h2>");
    // No bare TEXT input. A checkbox is exempt and always was — its
    // chrome is the platform's, which `check-classes.mjs` encodes for
    // the whole repo. The first version of this assertion said
    // `/<input\s/` and would have failed the moment a checkbox landed,
    // which is a lock on the wrong thing.
    expect(SOURCE).not.toMatch(/<input\s[^>]*type="(text|password)"/);
    expect(SOURCE.match(/<input\s/g) ?? []).toHaveLength(1);
  });
});

describe("RemotePane — the three states", () => {
  it("off is off", async () => {
    remoteStatusMock.mockResolvedValue(status({ enabled: false }));
    render(<RemotePane />);
    expect(await screen.findByText(t("remote.state.off"))).toBeInTheDocument();
  });

  /**
   * The state this pane exists for.
   *
   * `enabled` is a saved preference and survives a `kill -9`, so a pane
   * that rendered it as "Serving" would tell the user their phone can
   * reach this Mac while nothing is listening. `remote::approval`
   * heartbeats every 5s precisely because the preference is not
   * liveness; this is the same distinction on screen.
   */
  it("enabled with nothing serving is its own state, and says why it matters", async () => {
    remoteStatusMock.mockResolvedValue(status({ enabled: true, serving: false }));
    render(<RemotePane />);
    expect(await screen.findByText(t("remote.state.idle"))).toBeInTheDocument();
    expect(screen.getByText(t("remote.idleHint"))).toBeInTheDocument();
    expect(screen.queryByText(t("remote.state.serving"))).not.toBeInTheDocument();
  });

  it("serving is serving", async () => {
    remoteStatusMock.mockResolvedValue(
      status({ enabled: true, serving: true, runningHere: true, url: "https://x:8420" }),
    );
    render(<RemotePane />);
    expect(await screen.findByText(t("remote.state.serving"))).toBeInTheDocument();
    expect(screen.getByText("https://x:8420")).toBeInTheDocument();
  });
});

describe("RemotePane — two liveness fields", () => {
  /**
   * A `claudepot remote serve` in a terminal sets the heartbeat but is
   * not ours. Offering Stop for it would be a button that reports
   * success and changes nothing.
   */
  it("a server running outside this process gets no Stop button", async () => {
    remoteStatusMock.mockResolvedValue(
      status({ serving: true, runningHere: false, url: null }),
    );
    render(<RemotePane />);
    expect(await screen.findByText(t("remote.servingElsewhere"))).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: t("remote.stop") })).not.toBeInTheDocument();
  });

  it("a server we started gets Stop, not Start", async () => {
    remoteStatusMock.mockResolvedValue(
      status({ serving: true, runningHere: true, url: "https://x:8420" }),
    );
    render(<RemotePane />);
    expect(await screen.findByRole("button", { name: t("remote.stop") })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: t("remote.start") })).not.toBeInTheDocument();
  });
});

describe("RemotePane — disabled controls state their reason inline", () => {
  it("cannot start while disabled", async () => {
    remoteStatusMock.mockResolvedValue(status({ enabled: false }));
    render(<RemotePane />);
    expect(await screen.findByText(t("remote.cannotStartDisabled"))).toBeInTheDocument();
    expect(screen.getByRole("button", { name: t("remote.start") })).toBeDisabled();
  });

  it("cannot start with no password", async () => {
    remoteStatusMock.mockResolvedValue(status({ enabled: true, passwordSet: false }));
    render(<RemotePane />);
    expect(await screen.findByText(t("remote.cannotStartNoPassword"))).toBeInTheDocument();
    expect(screen.getByRole("button", { name: t("remote.start") })).toBeDisabled();
  });

  it("cannot enable with no password", async () => {
    remoteStatusMock.mockResolvedValue(status({ enabled: false, passwordSet: false }));
    render(<RemotePane />);
    expect(await screen.findByText(t("remote.cannotEnableNoPassword"))).toBeInTheDocument();
    expect(screen.getByRole("button", { name: t("remote.enable") })).toBeDisabled();
  });
});

describe("RemotePane — exposure", () => {
  /**
   * `0.0.0.0` is accepted deliberately and returned as
   * `every_interface` precisely so a caller has to say so. Core's words:
   * "Accepted, never silently."
   */
  it("binding every interface warns, and a private bind does not", async () => {
    remoteStatusMock.mockResolvedValue(
      status({ bind: "0.0.0.0", exposure: "every_interface" }),
    );
    const { unmount } = render(<RemotePane />);
    expect(await screen.findByText(t("remote.everyInterface"))).toBeInTheDocument();
    unmount();

    remoteStatusMock.mockResolvedValue(status({ exposure: "private_network" }));
    render(<RemotePane />);
    await screen.findByText(t("remote.state.idle"));
    expect(screen.queryByText(t("remote.everyInterface"))).not.toBeInTheDocument();
  });

  it("a refused address shows the refusal", async () => {
    remoteStatusMock.mockResolvedValue(
      status({ bind: "8.8.8.8", exposure: null, bindError: "publicly routable" }),
    );
    render(<RemotePane />);
    expect(await screen.findByText("publicly routable")).toBeInTheDocument();
  });
});

describe("RemotePane — the fail-loud stores", () => {
  it("a recovered device file blocks revoking and says why", async () => {
    remoteStatusMock.mockResolvedValue(
      status({
        devicesRecovered: true,
        activeDevices: 1,
        devices: [
          {
            id: "d1",
            name: "phone",
            createdAt: "2026-08-01T00:00:00Z",
            lastSeen: null,
            revokedAt: null,
            expiresAt: null,
          },
        ],
      }),
    );
    render(<RemotePane />);
    expect(await screen.findByText(t("remote.devicesRecovered"))).toBeInTheDocument();
    expect(screen.getByText(t("remote.cannotRevokeRecovered"))).toBeInTheDocument();
    expect(screen.getByRole("button", { name: t("remote.revokeAll") })).toBeDisabled();
    expect(screen.getByRole("button", { name: t("remote.revoke") })).toBeDisabled();
  });

  it("a recovered config file is announced", async () => {
    remoteStatusMock.mockResolvedValue(status({ configRecovered: true }));
    render(<RemotePane />);
    expect(await screen.findByText(t("remote.configRecovered"))).toBeInTheDocument();
  });
});

describe("RemotePane — the password", () => {
  /**
   * `rules/architecture.md`: renderer-side state must not outlive the
   * single bridge call. The field is cleared in the same tick the call
   * resolves.
   */
  it("clears the field once the password is saved", async () => {
    const user = userEvent.setup();
    remoteStatusMock.mockResolvedValue(status({ passwordSet: false, enabled: false }));
    remoteSetPasswordMock.mockResolvedValue(undefined);
    render(<RemotePane />);

    const field = await screen.findByLabelText(t("remote.newPassword"));
    await user.type(field, "hunter2");
    expect(field).toHaveValue("hunter2");

    await user.click(screen.getByRole("button", { name: t("remote.setPassword") }));
    await waitFor(() => expect(remoteSetPasswordMock).toHaveBeenCalledWith("hunter2"));
    await waitFor(() => expect(field).toHaveValue(""));
  });

  it("will not submit an empty password", async () => {
    remoteStatusMock.mockResolvedValue(status({ passwordSet: false }));
    render(<RemotePane />);
    expect(
      await screen.findByRole("button", { name: t("remote.setPassword") }),
    ).toBeDisabled();
  });
});

describe("RemotePane — the approvals toggle", () => {
  /**
   * The one capability on this surface that grants rather than reads.
   * It was hard-wired to the server's lifetime before this — starting
   * the server installed Claude Code's `PermissionRequest` hook, so
   * wanting the panel meant taking phone-approval with it.
   */
  it("reflects the stored preference", async () => {
    remoteStatusMock.mockResolvedValue(status({ approvalsEnabled: true }));
    const { unmount } = render(<RemotePane />);
    const on = await screen.findByLabelText(t("remote.approvalsLabel"));
    expect(on).toBeChecked();
    unmount();

    remoteStatusMock.mockResolvedValue(status({ approvalsEnabled: false }));
    render(<RemotePane />);
    expect(await screen.findByLabelText(t("remote.approvalsLabel"))).not.toBeChecked();
  });

  it("turning it off calls through with false", async () => {
    const user = userEvent.setup();
    remoteStatusMock.mockResolvedValue(status({ approvalsEnabled: true }));
    remoteSetApprovalsMock.mockResolvedValue(undefined);
    render(<RemotePane />);

    await user.click(await screen.findByLabelText(t("remote.approvalsLabel")));
    await waitFor(() => expect(remoteSetApprovalsMock).toHaveBeenCalledWith(false));
  });

  /**
   * Approving a tool call is arbitrary code execution as this user. The
   * consequence is stated at the control while it is on, and there is
   * nothing to warn about once it is off.
   */
  it("warns only while it is on", async () => {
    remoteStatusMock.mockResolvedValue(status({ approvalsEnabled: true }));
    const { unmount } = render(<RemotePane />);
    expect(await screen.findByText(t("remote.approvalsWarning"))).toBeInTheDocument();
    unmount();

    remoteStatusMock.mockResolvedValue(status({ approvalsEnabled: false }));
    render(<RemotePane />);
    await screen.findByLabelText(t("remote.approvalsLabel"));
    expect(screen.queryByText(t("remote.approvalsWarning"))).not.toBeInTheDocument();
  });
});

describe("RemotePane — approval warnings", () => {
  /**
   * Approval-from-the-phone being off while everything else works is
   * the failure a user would otherwise find by tapping Allow and
   * waiting for nothing to happen.
   */
  it("surfaces an approval-hook warning", async () => {
    remoteStatusMock.mockResolvedValue(
      status({
        serving: true,
        runningHere: true,
        warnings: ["remote approval is OFF — could not install the hook"],
      }),
    );
    render(<RemotePane />);
    expect(
      await screen.findByText("remote approval is OFF — could not install the hook"),
    ).toBeInTheDocument();
  });
});

describe("RemotePane — the in-process trade is disclosed", () => {
  /**
   * Quitting Claudepot stops the surface. That is the cost of hosting
   * in-process, and the pane has to say it rather than let the user
   * find out when their phone stops working.
   */
  it("says that quitting stops the surface", async () => {
    remoteStatusMock.mockResolvedValue(status());
    render(<RemotePane />);
    expect(await screen.findByText(t("remote.quitStops"))).toBeInTheDocument();
  });
});
