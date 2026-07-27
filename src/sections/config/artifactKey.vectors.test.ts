// Parity harness: `artifactKeyForFile` (TS) vs
// `claudepot_core::artifact_usage::identity::artifact_key_for_path` (Rust).
//
// Both sides run `crates/claudepot-core/testdata/artifact-key-vectors.json`.
// A divergence fails on exactly one side, which is the point — the two
// implementations join against the same usage ledger, so any drift makes
// "installed but never fired" silently wrong.
//
// Same contract as `testdata/rate-resolution-vectors.json` (AGENTS.md,
// "Pricing"): change one implementation, change the other, add a vector.

import { describe, expect, it } from "vitest";
// Direct JSON import (tsconfig `resolveJsonModule`) rather than fs —
// keeps the test free of node type deps and makes the dependency on the
// shared vector file explicit to the bundler.
import vectorDoc from "../../../crates/claudepot-core/testdata/artifact-key-vectors.json";
import { artifactKeyForFile } from "./artifactKey";
import type { ConfigFileNodeDto } from "../../types";

interface Vector {
  name: string;
  kind: string;
  path: string;
  project_root: string | null;
  expected: {
    kind: string;
    artifact_key: string;
    plugin_id: string | null;
  } | null;
}

function loadVectors(): Vector[] {
  return vectorDoc.vectors as Vector[];
}

function fileNode(kind: string, absPath: string): ConfigFileNodeDto {
  return {
    id: absPath,
    kind,
    abs_path: absPath,
    display_path: absPath,
    size_bytes: 1,
    mtime_unix_ns: 0,
    summary_title: null,
    summary_description: null,
    issues: [],
    included_by: null,
    include_depth: 0,
  } as ConfigFileNodeDto;
}

describe("artifactKey shared vectors (Rust ↔ TS parity)", () => {
  const vectors = loadVectors();

  it("loads a non-empty vector file", () => {
    expect(vectors.length).toBeGreaterThan(0);
  });

  for (const v of vectors) {
    it(v.name, () => {
      const got = artifactKeyForFile(fileNode(v.kind, v.path), v.project_root);
      if (v.expected === null) {
        expect(got).toBeNull();
        return;
      }
      expect(got).not.toBeNull();
      expect(got!.kind).toBe(v.expected.kind);
      expect(got!.artifactKey).toBe(v.expected.artifact_key);
      expect(got!.pluginId).toBe(v.expected.plugin_id);
    });
  }
});
