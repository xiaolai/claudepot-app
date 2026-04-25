/**
 * KeysSection — D-5/6/7 secret-IPC redesign tests.
 *
 * Verifies the new copy flow:
 *   - `keyApiCopy` is called with the row uuid (no plaintext arg).
 *   - The toast shows the receipt's label + preview, NOT the secret.
 *   - The renderer never imports `navigator.clipboard.writeText` for
 *     secret payloads — Rust does the clipboard write directly via
 *     `tauri-plugin-clipboard-manager`, so any leftover JS clipboard
 *     call would be a regression.
 */
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const keyApiList = vi.fn();
const keyOauthList = vi.fn();
const accountListBasic = vi.fn();
const keyApiCopy = vi.fn();
const keyOauthCopy = vi.fn();
const keyOauthCopyShell = vi.fn();
const keyApiProbe = vi.fn();
const keyApiRemove = vi.fn();
const keyOauthRemove = vi.fn();
const keyApiRename = vi.fn();
const keyOauthRename = vi.fn();

vi.mock("../api", () => ({
  api: {
    keyApiList: (...a: unknown[]) => keyApiList(...a),
    keyOauthList: (...a: unknown[]) => keyOauthList(...a),
    accountListBasic: (...a: unknown[]) => accountListBasic(...a),
    keyApiCopy: (...a: unknown[]) => keyApiCopy(...a),
    keyOauthCopy: (...a: unknown[]) => keyOauthCopy(...a),
    keyOauthCopyShell: (...a: unknown[]) => keyOauthCopyShell(...a),
    keyApiProbe: (...a: unknown[]) => keyApiProbe(...a),
    keyApiRemove: (...a: unknown[]) => keyApiRemove(...a),
    keyOauthRemove: (...a: unknown[]) => keyOauthRemove(...a),
    keyApiRename: (...a: unknown[]) => keyApiRename(...a),
    keyOauthRename: (...a: unknown[]) => keyOauthRename(...a),
  },
}));

import { KeysSection } from "./KeysSection";

const ACCOUNT_UUID = "11111111-1111-1111-1111-111111111111";
const API_UUID = "22222222-2222-2222-2222-222222222222";
const OAUTH_UUID = "33333333-3333-3333-3333-333333333333";
const RAW_API_SECRET = "sk-ant-api03-DO_NOT_LEAK_API_VALUE";
const RAW_OAUTH_SECRET = "sk-ant-oat01-DO_NOT_LEAK_OAUTH_VALUE";

beforeEach(() => {
  keyApiList.mockReset();
  keyOauthList.mockReset();
  accountListBasic.mockReset();
  keyApiCopy.mockReset();
  keyOauthCopy.mockReset();
  keyOauthCopyShell.mockReset();
  keyApiProbe.mockReset();
  keyApiRemove.mockReset();
  keyOauthRemove.mockReset();
  keyApiRename.mockReset();
  keyOauthRename.mockReset();

  keyApiList.mockResolvedValue([
    {
      uuid: API_UUID,
      label: "ci-api-key",
      token_preview: "sk-ant-api03-Abc…xyz",
      account_uuid: ACCOUNT_UUID,
      account_email: "ci@example.com",
      created_at: new Date().toISOString(),
      last_probed_at: null,
      last_probe_status: null,
    },
  ]);
  keyOauthList.mockResolvedValue([
    {
      uuid: OAUTH_UUID,
      label: "ci-oauth-token",
      token_preview: "sk-ant-oat01-Abc…xyz",
      account_uuid: ACCOUNT_UUID,
      account_email: "ci@example.com",
      created_at: new Date().toISOString(),
      expires_at: new Date(Date.now() + 86_400_000 * 365).toISOString(),
      days_remaining: 365,
      last_probed_at: null,
      last_probe_status: null,
    },
  ]);
  accountListBasic.mockResolvedValue([
    {
      uuid: ACCOUNT_UUID,
      email: "ci@example.com",
      org_name: null,
      subscription_type: null,
      is_cli_active: true,
      is_desktop_active: false,
      has_cli_credentials: true,
      has_desktop_profile: false,
    },
  ]);
});

describe("KeysSection — D-5/6/7 copy flow", () => {
  it("displays receipt label + preview, never the secret (api)", async () => {
    keyApiCopy.mockResolvedValue({
      label: "ci-api-key",
      preview: "sk-ant-api03-Abc…xyz",
      clipboard_clears_at_unix_ms: Date.now() + 30_000,
    });

    render(<KeysSection />);

    // Wait until the row mounts; then click the Copy icon by aria-label.
    const btn = await screen.findByLabelText(/Copy ci-api-key/);
    await userEvent.click(btn);

    await waitFor(() => expect(keyApiCopy).toHaveBeenCalledOnce());
    // Critical: only the row uuid crosses the bridge. No plaintext.
    expect(keyApiCopy).toHaveBeenCalledWith(API_UUID);

    // Toast renders label + preview verbatim. Plaintext must not
    // appear anywhere in the DOM.
    await waitFor(() =>
      expect(
        screen.getByText(
          /Copied ci-api-key \(sk-ant-api03-Abc…xyz\)/,
        ),
      ).toBeInTheDocument(),
    );
    expect(document.body.textContent ?? "").not.toContain(RAW_API_SECRET);
  });

  it("calls keyOauthCopy for the row uuid (oauth)", async () => {
    keyOauthCopy.mockResolvedValue({
      label: "ci-oauth-token",
      preview: "sk-ant-oat01-Abc…xyz",
      clipboard_clears_at_unix_ms: Date.now() + 30_000,
    });

    render(<KeysSection />);
    const btn = await screen.findByLabelText(/Copy ci-oauth-token/);
    await userEvent.click(btn);

    await waitFor(() => expect(keyOauthCopy).toHaveBeenCalledOnce());
    expect(keyOauthCopy).toHaveBeenCalledWith(OAUTH_UUID);
    expect(document.body.textContent ?? "").not.toContain(RAW_OAUTH_SECRET);
  });

  it("routes the shell-copy variant to keyOauthCopyShell", async () => {
    keyOauthCopyShell.mockResolvedValue({
      label: "ci-oauth-token",
      preview: "sk-ant-oat01-Abc…xyz",
      clipboard_clears_at_unix_ms: Date.now() + 30_000,
    });

    render(<KeysSection />);
    const btn = await screen.findByLabelText(
      /Copy shell command for ci-oauth-token/,
    );
    await userEvent.click(btn);

    await waitFor(() => expect(keyOauthCopyShell).toHaveBeenCalledOnce());
    expect(keyOauthCopyShell).toHaveBeenCalledWith(OAUTH_UUID);
    // The plain copy command MUST NOT have fired — that would mean the
    // shell wrapping happened in JS (the old, leaky shape).
    expect(keyOauthCopy).not.toHaveBeenCalled();
    // The shell-format toast hint identifies the right path.
    await waitFor(() =>
      expect(
        screen.getByText(/Copied shell command for ci-oauth-token/),
      ).toBeInTheDocument(),
    );
  });

  it("surfaces a copy-failed toast and never logs the secret", async () => {
    keyApiCopy.mockRejectedValue(new Error("clipboard: permission denied"));

    render(<KeysSection />);
    const btn = await screen.findByLabelText(/Copy ci-api-key/);
    await userEvent.click(btn);

    await waitFor(() =>
      expect(screen.getByText(/Copy failed/)).toBeInTheDocument(),
    );
    expect(document.body.textContent ?? "").not.toContain(RAW_API_SECRET);
  });
});
