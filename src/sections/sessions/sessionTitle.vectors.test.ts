import { describe, expect, it } from "vitest";
import { deriveSessionTitle } from "./format";
// Imported, not read from disk: `resolveJsonModule` types it, so a
// vector whose shape drifts is a compile error rather than a runtime
// surprise — and it needs no node types in a browser-targeted tsconfig.
import vectorsJson from "../../../crates/claudepot-core/testdata/session-title-vectors.json";

/**
 * The shared title vectors.
 *
 * `deriveSessionTitle` and `claudepot-core::session::title::derive` are
 * the same rule written twice — the panel gets its title from the Rust
 * one over HTTP, the desktop derives its own from `SessionRow` over IPC.
 * Two implementations drift unless something makes them agree, so both
 * run this file.
 *
 * The vectors are read from the crate's `testdata/` rather than copied
 * here on purpose: a copy is a cache of the original, and the point is
 * that there is exactly one list.
 */
interface Vector {
  name: string;
  raw: string;
  want: string | null;
}

const vectors = vectorsJson as Vector[];

describe("deriveSessionTitle — vectors shared with claudepot-core", () => {
  it("reads a non-trivial vector file", () => {
    // A file that failed to load, or lost its contents, would make every
    // case below pass by vacuum.
    expect(vectors.length).toBeGreaterThanOrEqual(20);
  });

  it.each(vectors.map((v) => [v.name, v] as const))("%s", (_name, v) => {
    expect(deriveSessionTitle(v.raw)).toBe(v.want);
  });

  it("returns null for a null prompt, which the vectors cannot express", () => {
    // The JSON has no way to distinguish "absent" from "empty string",
    // and the two reach this function differently: `first_user_prompt`
    // is nullable in the row.
    expect(deriveSessionTitle(null)).toBeNull();
  });
});
