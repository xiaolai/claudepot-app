import { describe, expect, it, vi } from "vitest";
import { extractMessage, renderError, toastError } from "./i18n-error";

describe("extractMessage", () => {
  it("passes strings through and unwraps Error.message", () => {
    expect(extractMessage("boom")).toBe("boom");
    expect(extractMessage(new Error("network down"))).toBe("network down");
  });

  it("reads a message property off plain objects", () => {
    expect(extractMessage({ message: "wrapped" })).toBe("wrapped");
  });

  it("never surfaces [object Object] or null", () => {
    expect(extractMessage({})).toBe("");
    expect(extractMessage(null)).toBe("");
    expect(extractMessage(undefined)).toBe("");
  });
});

describe("renderError", () => {
  it("prefixes the scope and stringifies non-Error values", () => {
    expect(renderError("boom", "Sync")).toBe("Sync: boom");
  });

  it("renders without a prefix when no scope is given", () => {
    expect(renderError(new Error("network down"))).toBe("network down");
  });

  it("redacts sk-ant- tokens that appear inside the error", () => {
    const e = new Error("server returned sk-ant-oat01-AbcdWxYz0000 oops");
    const out = renderError(e, "Adopt");
    expect(out).not.toContain("sk-ant-oat01-AbcdWxYz");
    expect(out).toContain("sk-ant-***0000");
  });

  it("truncates messages longer than 240 chars with an ellipsis", () => {
    const long = "x".repeat(500);
    const out = renderError(new Error(long), "Big");
    // scope + ": " + 240 budget = 245 max
    expect(out.length).toBeLessThanOrEqual("Big: ".length + 240);
    expect(out.endsWith("…")).toBe(true);
  });

  it("keeps short messages intact (no spurious ellipsis)", () => {
    const out = renderError(new Error("ok"), "Short");
    expect(out.endsWith("…")).toBe(false);
    expect(out).toBe("Short: ok");
  });
});

describe("renderError code resolution", () => {
  // These replace `lib/errors.test.ts`, deleted with the regex
  // remediation layer it covered. The guidance those regexes carried is
  // now catalog copy keyed off a real code, so the assertions here name
  // codes instead of matching English against English.

  it("renders the catalog sentence for a coded error, not its English", () => {
    const out = renderError({
      code: "shared_memory.index_unavailable",
      params: {},
      message: "session index unavailable (open failed at startup)",
    });
    // Was `/session index unavailable/i` in the deleted regex layer.
    expect(out).toMatch(/rebuild it from Settings/);
    expect(out).not.toMatch(/open failed at startup/);
  });

  it("keeps the moved-or-pruned guidance for an unreachable excerpt", () => {
    // Was `/no such file|not found|os error 2|\bmoved\b/i`. Rust now
    // classifies on the error variant, so the code — not the wording of
    // an io error — decides which sentence the user gets.
    const out = renderError({
      code: "shared_memory.excerpt_unavailable",
      params: { detail: "read: locator references unknown file_path: /x.jsonl" },
      message: "read: locator references unknown file_path: /x.jsonl",
    });
    expect(out).toMatch(/no longer available/);
  });

  it("interpolates params rather than re-parsing the English message", () => {
    const out = renderError({
      code: "shared_memory.invalid_kind",
      params: { kind: "opinion" },
      message: "invalid kind",
    });
    expect(out).toContain("opinion");
    expect(out).not.toContain("{{kind}}");
  });

  it("falls back to the English message when a code has no entry", () => {
    // The whole reason the Rust fan-out could land without a frontend
    // flag day: an unknown code degrades to what the user saw before.
    const out = renderError({
      code: "not_a_real.module_or_variant",
      params: {},
      message: "something specific from Rust",
    });
    expect(out).toBe("something specific from Rust");
  });

  it("still renders a legacy Result<T, String> rejection verbatim", () => {
    expect(renderError("list: no such table: memories")).toBe(
      "list: no such table: memories",
    );
  });

  it("floors an unrenderable value instead of rendering nothing", () => {
    // An empty string renders as a *missing* banner — the operation
    // failed and the surface said nothing. Inherited from the deleted
    // `toUserError`'s "Something went wrong." tail.
    expect(renderError({ nope: 1 })).toBe("Something went wrong.");
    expect(renderError(null)).toBe("Something went wrong.");
  });

  it("redacts tokens inside an interpolated catalog sentence", () => {
    const out = renderError({
      code: "shared_memory.search_failed",
      params: { detail: "search: rejected sk-ant-api01-aaaaaaaaaa1234" },
      message: "search: rejected sk-ant-api01-aaaaaaaaaa1234",
    });
    expect(out).not.toContain("aaaaaaaaaa1234");
    expect(out).toContain("sk-ant-***1234");
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
