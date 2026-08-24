import { describe, expect, it } from "vitest";
import { isSafeHref } from "./mdLink";

/**
 * Twin of the `isSafeHref` cases in `panel/src/app/markdown.test.js`.
 */
describe("isSafeHref", () => {
  it("allows the three protocols a transcript link may use", () => {
    expect(isSafeHref("https://example.com/x")).toBe(true);
    expect(isSafeHref("http://example.com")).toBe(true);
    expect(isSafeHref("mailto:someone@example.com")).toBe(true);
    expect(isSafeHref("HTTPS://EXAMPLE.COM")).toBe(true);
  });

  it("refuses the schemes react-markdown's default lets through", () => {
    // These are the gap: the library's transform blocks `javascript:`
    // but not these, and the desktop renderer hands the value to the
    // OS opener.
    expect(isSafeHref("irc://irc.example.com/chan")).toBe(false);
    expect(isSafeHref("ircs://irc.example.com/chan")).toBe(false);
    expect(isSafeHref("xmpp:someone@example.com")).toBe(false);
  });

  it("refuses javascript: and data:", () => {
    expect(isSafeHref("javascript:alert(1)")).toBe(false);
    expect(isSafeHref("  javascript:alert(1)")).toBe(false);
    expect(isSafeHref("data:text/html,<script>")).toBe(false);
    expect(isSafeHref("file:///etc/passwd")).toBe(false);
  });

  it("refuses relative URLs, which have no base inside a transcript", () => {
    expect(isSafeHref("/etc/passwd")).toBe(false);
    expect(isSafeHref("./notes.md")).toBe(false);
    expect(isSafeHref("notes.md")).toBe(false);
    expect(isSafeHref("//evil.example.com")).toBe(false);
  });

  it("refuses nothing at all", () => {
    expect(isSafeHref(undefined)).toBe(false);
    expect(isSafeHref("")).toBe(false);
  });
});
