import { describe, expect, it } from "vitest";
import { maybeRedact, redactSecrets } from "./redactSecrets";

describe("redactSecrets", () => {
  it("returns input unchanged when the needle is absent", () => {
    expect(redactSecrets("no secrets here")).toBe("no secrets here");
  });

  it("returns empty string unchanged", () => {
    expect(redactSecrets("")).toBe("");
  });

  it("masks a long sk-ant-oat token preserving last 4 chars", () => {
    const input = "leaked sk-ant-oat01-AbcdWxYz0000 keep going";
    const out = redactSecrets(input);
    expect(out).not.toContain("sk-ant-oat01-AbcdWxYz");
    expect(out).toContain("sk-ant-***0000");
  });

  it("masks a long sk-ant-api token preserving last 4 chars", () => {
    const out = redactSecrets("key sk-ant-api03-XYZwxyz9876 ok");
    expect(out).toContain("sk-ant-***9876");
  });

  it("fully masks a too-short token (no suffix exposure)", () => {
    // sk-ant-abc has total length 11, under the 12-char threshold.
    expect(redactSecrets("sk-ant-abc")).toBe("sk-ant-***");
  });

  it("masks multiple tokens in one string", () => {
    const out = redactSecrets(
      "first sk-ant-oat01-aaaaaaaaaa1111 then sk-ant-api03-bbbbbbbbbb2222 done",
    );
    expect(out).toContain("sk-ant-***1111");
    expect(out).toContain("sk-ant-***2222");
    expect(out).not.toContain("aaaaaaaaaa");
    expect(out).not.toContain("bbbbbbbbbb");
  });

  it("is idempotent on already-redacted text", () => {
    const once = redactSecrets("text sk-ant-oat01-Abcdefghijklmnop1234 end");
    const twice = redactSecrets(once);
    expect(twice).toBe(once);
    // Critically, no doubling — should not produce sk-ant-******1234.
    expect(twice).not.toContain("******");
  });

  it("preserves text around the token verbatim", () => {
    const out = redactSecrets("before sk-ant-oat01-zzzzzzzzzzz9999 after");
    expect(out).toBe("before sk-ant-***9999 after");
  });

  it("does not match prefixes that aren't the needle", () => {
    // Dash-letter mismatches: only the literal "sk-ant-" triggers.
    expect(redactSecrets("sk_ant_oat01-aaaaaa1234")).toBe(
      "sk_ant_oat01-aaaaaa1234",
    );
    expect(redactSecrets("Sk-Ant-oat01-aaaaaa1234")).toBe(
      "Sk-Ant-oat01-aaaaaa1234",
    );
  });

  it("handles a token at the end of the string", () => {
    const out = redactSecrets("trailing sk-ant-oat01-zzzzzzzzzzz9999");
    expect(out).toBe("trailing sk-ant-***9999");
  });

  it("handles a token at the start of the string", () => {
    const out = redactSecrets("sk-ant-oat01-zzzzzzzzzzz9999 leading");
    expect(out).toBe("sk-ant-***9999 leading");
  });

  it("does not run away on a bare sk-ant- prefix with no token chars after", () => {
    expect(redactSecrets("sk-ant-")).toBe("sk-ant-***");
  });
});

describe("maybeRedact", () => {
  it("passes null through", () => {
    expect(maybeRedact(null)).toBeNull();
  });
  it("passes undefined through", () => {
    expect(maybeRedact(undefined)).toBeUndefined();
  });
  it("redacts strings", () => {
    expect(maybeRedact("sk-ant-oat01-aaaaaaaaaa1234")).toBe("sk-ant-***1234");
  });
});
