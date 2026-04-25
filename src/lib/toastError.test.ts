import { describe, expect, it, vi } from "vitest";
import { formatErrorMessage, toastError } from "./toastError";

describe("formatErrorMessage", () => {
  it("prefixes the scope and stringifies non-Error values", () => {
    expect(formatErrorMessage("Sync", "boom")).toBe("Sync: boom");
  });

  it("uses Error.message for Error instances", () => {
    const e = new Error("network down");
    expect(formatErrorMessage("Verify", e)).toBe("Verify: network down");
  });

  it("redacts sk-ant- tokens that appear inside the error", () => {
    const e = new Error("server returned sk-ant-oat01-AbcdWxYz0000 oops");
    const out = formatErrorMessage("Adopt", e);
    expect(out).not.toContain("sk-ant-oat01-AbcdWxYz");
    expect(out).toContain("sk-ant-***0000");
  });

  it("truncates messages longer than 240 chars with an ellipsis", () => {
    const long = "x".repeat(500);
    const out = formatErrorMessage("Big", new Error(long));
    // scope + ": " + 240 budget = 247 max
    expect(out.length).toBeLessThanOrEqual("Big: ".length + 240);
    expect(out.endsWith("…")).toBe(true);
  });

  it("keeps short messages intact (no spurious ellipsis)", () => {
    const out = formatErrorMessage("Short", new Error("ok"));
    expect(out.endsWith("…")).toBe(false);
    expect(out).toBe("Short: ok");
  });
});

describe("toastError", () => {
  it("calls pushToast with kind=error and a redacted message", () => {
    const push = vi.fn();
    toastError(push, "Login", new Error("creds=sk-ant-api01-aaaaaaaaaa1234 bad"));
    expect(push).toHaveBeenCalledTimes(1);
    expect(push.mock.calls[0][0]).toBe("error");
    const text = push.mock.calls[0][1] as string;
    expect(text.startsWith("Login: ")).toBe(true);
    expect(text).not.toContain("aaaaaaaaaa1234");
    expect(text).toContain("sk-ant-***1234");
  });
});
