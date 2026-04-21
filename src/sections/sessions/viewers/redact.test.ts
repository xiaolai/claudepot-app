import { describe, expect, it } from "vitest";
import { redactSecrets } from "./redact";

describe("redactSecrets (UI mirror of core)", () => {
  it("masks a standard sk-ant-oat token", () => {
    const out = redactSecrets("key sk-ant-oat01-Abcdefghijkl1234 tail");
    expect(out).toContain("sk-ant-***1234");
    expect(out).not.toContain("sk-ant-oat01-Abcdefghijkl");
  });

  it("masks multiple tokens in one string", () => {
    const out = redactSecrets("a sk-ant-api01-Aaaa0000 b sk-ant-api02-Bbbb1111");
    expect(out).toContain("sk-ant-***0000");
    expect(out).toContain("sk-ant-***1111");
  });

  it("leaves non-secret text untouched", () => {
    const out = redactSecrets("no secrets here, just text");
    expect(out).toBe("no secrets here, just text");
  });

  it("handles null and undefined gracefully", () => {
    expect(redactSecrets(null)).toBe("");
    expect(redactSecrets(undefined)).toBe("");
  });

  it("collapses short tokens completely", () => {
    expect(redactSecrets("sk-ant-x")).toBe("sk-ant-***");
  });
});
