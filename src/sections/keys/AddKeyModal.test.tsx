/**
 * AddKeyModal — D-5/6/7 secret-IPC redesign tests.
 *
 * Verifies that the token field is scrubbed from React state on
 * submit, regardless of whether the backend call succeeded or
 * threw. The plaintext is needed for exactly one bridge call; any
 * lingering state would leave it observable via DevTools React
 * inspector or any subsequent re-render.
 */
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const keyApiAdd = vi.fn();
const keyOauthAdd = vi.fn();

vi.mock("../../api", () => ({
  api: {
    keyApiAdd: (...a: unknown[]) => keyApiAdd(...a),
    keyOauthAdd: (...a: unknown[]) => keyOauthAdd(...a),
  },
}));

import { AddKeyModal } from "./AddKeyModal";
import type { AccountSummaryBasic } from "../../types";

const ACCOUNT: AccountSummaryBasic = {
  uuid: "11111111-1111-1111-1111-111111111111",
  email: "ci@example.com",
  org_name: null,
  subscription_type: null,
  is_cli_active: true,
  is_desktop_active: false,
  has_cli_credentials: true,
  has_desktop_profile: false,
};

beforeEach(() => {
  keyApiAdd.mockReset();
  keyOauthAdd.mockReset();
});

async function fillFormAndSubmit(token: string) {
  const onClose = vi.fn();
  const onAdded = vi.fn();
  render(
    <AddKeyModal
      accounts={[ACCOUNT]}
      onClose={onClose}
      onAdded={onAdded}
    />,
  );

  const labelInput = screen.getByPlaceholderText(
    /Personal \/ CI \/ staging/,
  ) as HTMLInputElement;
  await userEvent.type(labelInput, "ci-key");

  // The token field has type=password — find it by its placeholder so
  // we don't get confused with the label field.
  const tokenInput = screen.getByPlaceholderText(
    /sk-ant-api03-… or sk-ant-oat01-…/,
  ) as HTMLInputElement;
  await userEvent.type(tokenInput, token);

  await userEvent.click(screen.getByRole("button", { name: /^Add$/ }));

  return { onAdded, onClose, tokenInput };
}

describe("AddKeyModal — D-5/6/7 secret hygiene", () => {
  it("clears token state on submit success (api key path)", async () => {
    keyApiAdd.mockResolvedValue({});
    const token = "sk-ant-api03-success-secret-12345";
    const { tokenInput, onAdded } = await fillFormAndSubmit(token);

    await waitFor(() => expect(keyApiAdd).toHaveBeenCalledOnce());
    expect(keyApiAdd).toHaveBeenCalledWith(
      "ci-key",
      token,
      ACCOUNT.uuid,
    );
    // onAdded was called → the modal would normally unmount, but the
    // test harness keeps it rendered. Either way the input must be
    // empty: production code unmounts it; tests assert the scrub.
    await waitFor(() => expect(onAdded).toHaveBeenCalledWith("api"));
    expect(tokenInput.value).toBe("");
  });

  it("clears token state on submit error (api key path)", async () => {
    keyApiAdd.mockRejectedValue(new Error("backend rejected"));
    const token = "sk-ant-api03-error-secret-67890";
    const { tokenInput, onAdded } = await fillFormAndSubmit(token);

    await waitFor(() => expect(keyApiAdd).toHaveBeenCalledOnce());
    // onAdded must NOT fire on error — the row didn't land.
    expect(onAdded).not.toHaveBeenCalled();
    // …but the token state must still be scrubbed regardless.
    await waitFor(() => expect(tokenInput.value).toBe(""));
    // The DOM must not display the raw secret anywhere — this guards
    // against a lazy refactor that leaks the token into the error
    // toast / banner.
    expect(document.body.textContent ?? "").not.toContain(token);
  });

  it("clears token state on submit success (oauth path)", async () => {
    keyOauthAdd.mockResolvedValue({});
    const token = "sk-ant-oat01-oauth-success-token-abcdef";
    const { tokenInput, onAdded } = await fillFormAndSubmit(token);

    await waitFor(() => expect(keyOauthAdd).toHaveBeenCalledOnce());
    expect(keyOauthAdd).toHaveBeenCalledWith(
      "ci-key",
      token,
      ACCOUNT.uuid,
    );
    await waitFor(() => expect(onAdded).toHaveBeenCalledWith("oauth"));
    expect(tokenInput.value).toBe("");
  });

  it("clears token state on submit error (oauth path)", async () => {
    keyOauthAdd.mockRejectedValue(new Error("rejected"));
    const token = "sk-ant-oat01-oauth-error-token-zzz";
    const { tokenInput, onAdded } = await fillFormAndSubmit(token);

    await waitFor(() => expect(keyOauthAdd).toHaveBeenCalledOnce());
    expect(onAdded).not.toHaveBeenCalled();
    await waitFor(() => expect(tokenInput.value).toBe(""));
    expect(document.body.textContent ?? "").not.toContain(token);
  });
});
