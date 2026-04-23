import { describe, expect, it, vi } from "vitest";
import { renderHook } from "@testing-library/react";
import { useStatusIssues } from "./useStatusIssues";
import { sampleAccount, sampleStatus } from "../test/fixtures";
import type { CcIdentity } from "../types";

const ccIdentity = (email: string): CcIdentity => ({
  email,
  verified_at: "2026-04-21T00:00:00Z",
  error: null,
});

describe("useStatusIssues — cc-slot drift", () => {
  it("resolves to a registered account → single 'Open matching account' action", () => {
    const onSelectAccount = vi.fn();
    const onReloginActive = vi.fn();
    const onImportCurrent = vi.fn();
    const alice = sampleAccount({
      email: "alice@example.com",
      is_cli_active: false,
    });
    const { result } = renderHook(() =>
      useStatusIssues({
        ccIdentity: ccIdentity("alice@example.com"),
        status: sampleStatus({ cli_active_email: "bob@example.com" }),
        syncError: null,
        keychainIssue: null,
        accounts: [alice, sampleAccount({ email: "bob@example.com" })],
        onUnlock: () => {},
        onSelectAccount,
        onReloginActive,
        onImportCurrent,
      }),
    );
    const drift = result.current.find((i) => i.id.startsWith("cc-drift:"));
    expect(drift).toBeDefined();
    expect(drift!.action?.label).toBe("Open matching account");
    expect(drift!.action2).toBeUndefined();
    drift!.action?.onClick();
    expect(onSelectAccount).toHaveBeenCalledWith(alice.uuid);
    expect(onImportCurrent).not.toHaveBeenCalled();
    expect(onReloginActive).not.toHaveBeenCalled();
  });

  it("unknown CC email → primary 'Import {email}' + secondary 'Re-login active'", () => {
    const onImportCurrent = vi.fn();
    const onReloginActive = vi.fn();
    const { result } = renderHook(() =>
      useStatusIssues({
        ccIdentity: ccIdentity("lixiaolai@gmail.com"),
        status: sampleStatus({ cli_active_email: "xiaolaiapple@gmail.com" }),
        syncError: null,
        keychainIssue: null,
        accounts: [sampleAccount({ email: "xiaolaiapple@gmail.com" })],
        onUnlock: () => {},
        onReloginActive,
        onImportCurrent,
      }),
    );
    const drift = result.current.find((i) => i.id.startsWith("cc-drift:"));
    expect(drift).toBeDefined();
    expect(drift!.action?.label).toBe("Import lixiaolai@gmail.com");
    expect(drift!.action2?.label).toBe("Re-login active");
    drift!.action?.onClick();
    expect(onImportCurrent).toHaveBeenCalledWith("lixiaolai@gmail.com");
    drift!.action2?.onClick();
    expect(onReloginActive).toHaveBeenCalled();
  });

  it("unknown CC email, only re-login wired → promotes it to primary (no action2)", () => {
    const onReloginActive = vi.fn();
    const { result } = renderHook(() =>
      useStatusIssues({
        ccIdentity: ccIdentity("lixiaolai@gmail.com"),
        status: sampleStatus({ cli_active_email: "xiaolaiapple@gmail.com" }),
        syncError: null,
        keychainIssue: null,
        accounts: [sampleAccount({ email: "xiaolaiapple@gmail.com" })],
        onUnlock: () => {},
        onReloginActive,
      }),
    );
    const drift = result.current.find((i) => i.id.startsWith("cc-drift:"));
    expect(drift!.action?.label).toBe("Re-login active");
    expect(drift!.action2).toBeUndefined();
  });

  it("email equality ignores case (no drift banner when CC == slot case-insensitive)", () => {
    const { result } = renderHook(() =>
      useStatusIssues({
        ccIdentity: ccIdentity("Alice@Example.com"),
        status: sampleStatus({ cli_active_email: "alice@example.com" }),
        syncError: null,
        keychainIssue: null,
        accounts: [sampleAccount({ email: "alice@example.com" })],
        onUnlock: () => {},
      }),
    );
    expect(
      result.current.find((i) => i.id.startsWith("cc-drift:")),
    ).toBeUndefined();
  });

  it("unique-prefix match resolves to 'Open matching account' (no import)", () => {
    const onSelectAccount = vi.fn();
    const onImportCurrent = vi.fn();
    const alice = sampleAccount({ email: "alice@example.com" });
    const { result } = renderHook(() =>
      useStatusIssues({
        // CC reports a prefix of the registered email — the hook's
        // resolve_email-style unique-prefix match should pick Alice.
        ccIdentity: ccIdentity("alice@"),
        status: sampleStatus({ cli_active_email: "bob@example.com" }),
        syncError: null,
        keychainIssue: null,
        accounts: [alice, sampleAccount({ email: "bob@example.com" })],
        onUnlock: () => {},
        onSelectAccount,
        onImportCurrent,
      }),
    );
    const drift = result.current.find((i) => i.id.startsWith("cc-drift:"));
    expect(drift!.action?.label).toBe("Open matching account");
    drift!.action?.onClick();
    expect(onSelectAccount).toHaveBeenCalledWith(alice.uuid);
    expect(onImportCurrent).not.toHaveBeenCalled();
  });

  it("ambiguous prefix → treated as unknown (offers Import + Re-login)", () => {
    const onImportCurrent = vi.fn();
    const onReloginActive = vi.fn();
    const { result } = renderHook(() =>
      useStatusIssues({
        // "al" is a prefix of both alice and alan → ambiguous, so
        // target is undefined and we fall into the unknown branch.
        ccIdentity: ccIdentity("al"),
        status: sampleStatus({ cli_active_email: "bob@example.com" }),
        syncError: null,
        keychainIssue: null,
        accounts: [
          sampleAccount({ email: "alice@example.com" }),
          sampleAccount({
            email: "alan@example.com",
            uuid: "bbbb2222-3333-4444-8555-666666666666",
          }),
          sampleAccount({
            email: "bob@example.com",
            uuid: "cccc3333-4444-4555-8666-777777777777",
          }),
        ],
        onUnlock: () => {},
        onImportCurrent,
        onReloginActive,
      }),
    );
    const drift = result.current.find((i) => i.id.startsWith("cc-drift:"));
    expect(drift!.action?.label).toBe("Import al");
    expect(drift!.action2?.label).toBe("Re-login active");
  });

  it("resolved target without onSelectAccount → no import fallback (banner action-less)", () => {
    // Regression guard: when CC's email matches a registered
    // account but the consumer didn't wire onSelectAccount (tests,
    // or an embedding without a sidebar), we must NOT fall through
    // to "Import {email}" — that would register the email a second
    // time.
    const onImportCurrent = vi.fn();
    const onReloginActive = vi.fn();
    const { result } = renderHook(() =>
      useStatusIssues({
        ccIdentity: ccIdentity("alice@example.com"),
        status: sampleStatus({ cli_active_email: "bob@example.com" }),
        syncError: null,
        keychainIssue: null,
        accounts: [
          sampleAccount({ email: "alice@example.com" }),
          sampleAccount({
            email: "bob@example.com",
            uuid: "bbbb2222-3333-4444-8555-666666666666",
          }),
        ],
        onUnlock: () => {},
        // onSelectAccount intentionally omitted.
        onImportCurrent,
        onReloginActive,
      }),
    );
    const drift = result.current.find((i) => i.id.startsWith("cc-drift:"));
    expect(drift).toBeDefined();
    expect(drift!.action).toBeUndefined();
    expect(drift!.action2).toBeUndefined();
    expect(onImportCurrent).not.toHaveBeenCalled();
    expect(onReloginActive).not.toHaveBeenCalled();
  });
});

// ---------------------------------------------------------------------------
// Tier 2-C: Desktop-side banner smoke tests.
// Exercises the `desktopSync` code path added in Phase 4. Covers all
// five DesktopSyncOutcome variants so a regression in the banner
// wiring is caught mechanically.
// ---------------------------------------------------------------------------

describe("useStatusIssues — desktop sync banners", () => {
  const baseOpts = {
    ccIdentity: null,
    status: sampleStatus(),
    syncError: null,
    keychainIssue: null,
    accounts: [] as ReturnType<typeof sampleAccount>[],
    onUnlock: () => {},
  };

  it("adoption_available → info banner with Bind action", () => {
    const onAdoptLiveDesktop = vi.fn();
    const { result } = renderHook(() =>
      useStatusIssues({
        ...baseOpts,
        desktopSync: { kind: "adoption_available", email: "alice@example.com" },
        onAdoptLiveDesktop,
      }),
    );
    const banner = result.current.find((i) => i.id.startsWith("desktop-adopt:"));
    expect(banner).toBeDefined();
    expect(banner!.severity).toBe("info");
    expect(banner!.label).toContain("alice@example.com");
    expect(banner!.action?.label).toBe("Bind");
    banner!.action!.onClick();
    expect(onAdoptLiveDesktop).toHaveBeenCalledWith("alice@example.com");
  });

  it("stranger → info banner with Import action", () => {
    const onImportDesktop = vi.fn();
    const { result } = renderHook(() =>
      useStatusIssues({
        ...baseOpts,
        desktopSync: { kind: "stranger", email: "new@example.com" },
        onImportDesktop,
      }),
    );
    const banner = result.current.find((i) => i.id.startsWith("desktop-stranger:"));
    expect(banner).toBeDefined();
    expect(banner!.severity).toBe("info");
    expect(banner!.action?.label).toBe("Import new@example.com");
    banner!.action!.onClick();
    expect(onImportDesktop).toHaveBeenCalledWith("new@example.com");
  });

  it("candidate_only → warning banner with NO mutating action", () => {
    // Critical property — Codex D5-1 / D5-2: fast-path candidate must
    // never drive mutation. The banner is advisory only.
    const onAdoptLiveDesktop = vi.fn();
    const onImportDesktop = vi.fn();
    const { result } = renderHook(() =>
      useStatusIssues({
        ...baseOpts,
        desktopSync: { kind: "candidate_only", email: "maybe@example.com" },
        onAdoptLiveDesktop,
        onImportDesktop,
      }),
    );
    const banner = result.current.find((i) => i.id.startsWith("desktop-candidate:"));
    expect(banner).toBeDefined();
    expect(banner!.severity).toBe("warning");
    expect(banner!.action).toBeUndefined();
    expect(banner!.action2).toBeUndefined();
    expect(onAdoptLiveDesktop).not.toHaveBeenCalled();
    expect(onImportDesktop).not.toHaveBeenCalled();
  });

  it("verified → no Desktop banner", () => {
    const { result } = renderHook(() =>
      useStatusIssues({
        ...baseOpts,
        desktopSync: { kind: "verified", email: "active@example.com" },
      }),
    );
    const desktop = result.current.filter((i) => i.id.startsWith("desktop-"));
    expect(desktop).toEqual([]);
  });

  it("no_live → no Desktop banner", () => {
    const { result } = renderHook(() =>
      useStatusIssues({
        ...baseOpts,
        desktopSync: { kind: "no_live" },
      }),
    );
    const desktop = result.current.filter((i) => i.id.startsWith("desktop-"));
    expect(desktop).toEqual([]);
  });

  it("banner is dismissable so the 24h snooze store can suppress it", () => {
    const { result } = renderHook(() =>
      useStatusIssues({
        ...baseOpts,
        desktopSync: { kind: "adoption_available", email: "x@example.com" },
      }),
    );
    const banner = result.current.find((i) => i.id.startsWith("desktop-adopt:"));
    expect(banner?.dismissable).toBe(true);
  });
});
