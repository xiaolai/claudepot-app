import { describe, expect, it } from "vitest";
import type {
  LinkedTool,
  SessionChunk,
  SessionEvent,
  SessionRow,
} from "../../types";
import {
  chunkMatchesSearch,
  classifyMetaMatch,
  eventMatchesSearch,
  normalizeDetailQuery,
} from "./sessionDetail.search";

/**
 * classifyMetaMatch powers the detail viewer's "your query matched on
 * row metadata, not transcript content" banner. The row shape carries
 * a lot of fields we don't care about here — build a minimal fixture
 * and override just the meta fields each case exercises.
 */
function row(overrides: Partial<SessionRow> = {}): SessionRow {
  return {
    session_id: "00000000-0000-0000-0000-000000000000",
    slug: "-r",
    file_path: "/sess.jsonl",
    file_size_bytes: 0,
    last_modified_ms: null,
    project_path: "/repo",
    project_from_transcript: true,
    first_ts: null,
    last_ts: null,
    event_count: 0,
    message_count: 0,
    user_message_count: 0,
    assistant_message_count: 0,
    first_user_prompt: null,
    models: [],
    tokens: {
      input: 0,
      output: 0,
      cache_creation: 0,
      cache_read: 0,
      total: 0,
    },
    git_branch: null,
    cc_version: null,
    display_slug: null,
    has_error: false,
    is_sidechain: false,
    ...overrides,
  };
}

describe("classifyMetaMatch", () => {
  it("returns empty for an empty query", () => {
    const matches = classifyMetaMatch(
      row({ project_path: "/Users/joker/src-tauri/app" }),
      "",
    );
    expect(matches).toEqual([]);
  });

  it("matches on project_path and reports the field label", () => {
    const matches = classifyMetaMatch(
      row({ project_path: "/Users/joker/src-tauri/app" }),
      "tauri",
    );
    expect(matches).toHaveLength(1);
    expect(matches[0].field).toBe("project path");
    expect(matches[0].value).toBe("/Users/joker/src-tauri/app");
  });

  it("matches on git_branch", () => {
    const matches = classifyMetaMatch(
      row({
        project_path: "/x",
        git_branch: "feat/tauri-upgrade",
      }),
      "tauri",
    );
    expect(matches.map((m) => m.field)).toEqual(["branch"]);
    expect(matches[0].value).toBe("feat/tauri-upgrade");
  });

  it("matches on any model and surfaces the matching entry", () => {
    const matches = classifyMetaMatch(
      row({
        project_path: "/x",
        models: ["claude-opus-4-7", "claude-sonnet-4-6"],
      }),
      "sonnet",
    );
    expect(matches).toHaveLength(1);
    expect(matches[0].field).toBe("model");
    expect(matches[0].value).toBe("claude-sonnet-4-6");
  });

  it("matches session_id via case-insensitive prefix only, not substring", () => {
    const id = "ABC12345-0000-0000-0000-000000000000";
    // Prefix hit.
    expect(
      classifyMetaMatch(row({ session_id: id }), "abc").map((m) => m.field),
    ).toEqual(["session id"]);
    // Substring inside the id, but not a prefix — rejected.
    expect(
      classifyMetaMatch(row({ session_id: id }), "12345"),
    ).toEqual([]);
  });

  it("reports multiple matching fields when several meta fields carry the query", () => {
    const matches = classifyMetaMatch(
      row({
        project_path: "/Users/joker/src-tauri/app",
        git_branch: "feat/tauri",
        models: ["claude-tauri-preview"],
      }),
      "tauri",
    );
    expect(matches.map((m) => m.field)).toEqual([
      "project path",
      "branch",
      "model",
    ]);
  });

  it("is case-insensitive on free-text fields", () => {
    const matches = classifyMetaMatch(
      row({ project_path: "/Users/joker/SRC-TAURI/app" }),
      "tauri",
    );
    expect(matches).toHaveLength(1);
    expect(matches[0].field).toBe("project path");
  });

  it("ignores null git_branch and empty models without throwing", () => {
    expect(
      classifyMetaMatch(row({ git_branch: null, models: [] }), "x"),
    ).toEqual([]);
  });

  it("treats secret-shaped meta values as literal strings (redaction happens at render time)", () => {
    // The classifier itself is the wrong layer to scrub secrets —
    // that's the renderer's job. We still want to verify that it
    // doesn't silently drop a match when the meta value looks
    // sensitive, so the banner ALWAYS surfaces in the empty state and
    // the UI layer can make the redact/render decision.
    const matches = classifyMetaMatch(
      row({ project_path: "/tmp/sk-ant-leak/project" }),
      "leak",
    );
    expect(matches).toHaveLength(1);
    expect(matches[0].field).toBe("project path");
    expect(matches[0].value).toContain("sk-ant-leak");
  });

  it("survives a row from an older Tauri binary where models is null", () => {
    // In the TS type `models` is `string[]`, but an older backend
    // build can serialize it as `null`. The classifier must coerce
    // before calling `.find` rather than blowing up a `useMemo`.
    const legacyRow = row({ models: undefined as unknown as string[] });
    expect(() => classifyMetaMatch(legacyRow, "anything")).not.toThrow();
    expect(classifyMetaMatch(legacyRow, "anything")).toEqual([]);
  });
});

/**
 * The detail search predicate changed from scanning `input_preview`
 * (240-char truncated display form) to `input_full` (raw JSON).
 * These tests pin that switch: a match past the preview cap must be
 * reachable, and an older bundle that ships no `input_full` must
 * still work via the preview fallback.
 */
describe("eventMatchesSearch / assistantToolUse", () => {
  const makeToolUse = (
    overrides: Partial<Extract<SessionEvent, { kind: "assistantToolUse" }>>,
  ): SessionEvent => ({
    kind: "assistantToolUse",
    ts: null,
    uuid: null,
    model: null,
    tool_name: "Bash",
    tool_use_id: "tu1",
    input_preview: "{}",
    input_full: "{}",
    ...overrides,
  });

  it("matches a term that lives past the 240-char preview cap", () => {
    const head = "x".repeat(260);
    const ev = makeToolUse({
      input_preview: `{"command":"${head.slice(0, 230)}`, // truncated
      input_full: `{"command":"${head}TAURI_DEEP"}`,
    });
    expect(eventMatchesSearch(ev, "tauri_deep")).toBe(true);
  });

  it("falls back to input_preview when input_full is empty (older bundle)", () => {
    const ev = makeToolUse({
      input_preview: '{"cmd":"pnpm tauri dev"}',
      input_full: "",
    });
    expect(eventMatchesSearch(ev, "tauri")).toBe(true);
  });

  it("falls back to input_preview when input_full is missing entirely", () => {
    // Casting through `unknown` to simulate a wire shape from an
    // older binary that doesn't know about `input_full`. This is the
    // exact class of payload `safeLower` has to absorb.
    const legacy = {
      kind: "assistantToolUse",
      ts: null,
      uuid: null,
      model: null,
      tool_name: "Bash",
      tool_use_id: "tu1",
      input_preview: '{"cmd":"pnpm tauri dev"}',
      // input_full intentionally absent
    } as unknown as SessionEvent;
    expect(() => eventMatchesSearch(legacy, "tauri")).not.toThrow();
    expect(eventMatchesSearch(legacy, "tauri")).toBe(true);
  });

  it("matches on tool_name independent of the input payload", () => {
    const ev = makeToolUse({
      tool_name: "TauriCustom",
      input_preview: "{}",
      input_full: "{}",
    });
    expect(eventMatchesSearch(ev, "tauri")).toBe(true);
  });
});

describe("chunkMatchesSearch", () => {
  const toolUse = (
    input_full: string,
  ): Extract<SessionEvent, { kind: "assistantToolUse" }> => ({
    kind: "assistantToolUse",
    ts: null,
    uuid: null,
    model: null,
    tool_name: "Bash",
    tool_use_id: "tu1",
    input_preview: input_full.slice(0, 40),
    input_full,
  });

  const toolResult = (
    content: string,
  ): Extract<SessionEvent, { kind: "userToolResult" }> => ({
    kind: "userToolResult",
    ts: null,
    uuid: null,
    tool_use_id: "tu1",
    content,
    is_error: false,
  });

  // Build the minimum viable AI chunk shape the search predicate
  // reads. Fields not under test are filled with defaults that don't
  // affect the matcher (metrics, timestamps, tool_executions).
  const emptyLinked: LinkedTool[] = [];
  const aiChunk = (event_indices: number[]): SessionChunk => ({
    id: 1,
    chunkType: "ai",
    event_indices,
    tool_executions: emptyLinked,
    start_ts: null,
    end_ts: null,
    metrics: {
      duration_ms: 0,
      tokens: {
        input: 0,
        output: 0,
        cache_creation: 0,
        cache_read: 0,
        total: 0,
      },
      message_count: 0,
      tool_call_count: 0,
      thinking_count: 0,
    },
  });

  it("finds a match reached only via the event_indices scan", () => {
    const events: SessionEvent[] = [
      toolUse(`{"command":"pnpm tauri dev","description":"start"}`),
    ];
    const chunk = aiChunk([0]);
    expect(chunkMatchesSearch(chunk, events, "tauri")).toBe(true);
  });

  it("does not double-count when the same term lives in both the call and its result", () => {
    // The chunk's event_indices covers the tool_use AND its
    // tool_result pair. The older version of `chunkMatchesSearch`
    // also looped over `tool_executions` separately — that duplicate
    // loop was the performance finding we just closed. The test
    // mirrors that scenario: same term appears in both events,
    // matcher should still return true (boolean short-circuit) but
    // importantly, no crash if `tool_executions` is absent.
    const events: SessionEvent[] = [
      toolUse(`{"command":"echo tauri"}`),
      toolResult("tauri stdout"),
    ];
    const chunk = aiChunk([0, 1]);
    expect(chunkMatchesSearch(chunk, events, "tauri")).toBe(true);
  });

  it("returns false when the chunk has no events and no linked tools", () => {
    const chunk = aiChunk([]);
    expect(chunkMatchesSearch(chunk, [], "tauri")).toBe(false);
  });
});

/**
 * `normalizeDetailQuery` is the one place trim+case normalization
 * happens. An earlier bug had two call sites doing it differently —
 * that ghost is exactly what these tests are pinning.
 */
describe("normalizeDetailQuery", () => {
  it("returns null for an empty string", () => {
    expect(normalizeDetailQuery("")).toBeNull();
  });

  it("returns null for whitespace-only input", () => {
    expect(normalizeDetailQuery("   ")).toBeNull();
    expect(normalizeDetailQuery("\t\n")).toBeNull();
  });

  it("returns null for single-character trimmed input", () => {
    // 2-char floor matches the list-level search and the Rust
    // SearchQuery::new guard.
    expect(normalizeDetailQuery("x")).toBeNull();
    expect(normalizeDetailQuery("  x  ")).toBeNull();
  });

  it("trims edge whitespace before length-checking and casing", () => {
    expect(normalizeDetailQuery("  tauri  ")).toBe("tauri");
    expect(normalizeDetailQuery("\tTAURI\n")).toBe("tauri");
  });

  it("lowercases the result", () => {
    expect(normalizeDetailQuery("TAURI")).toBe("tauri");
    expect(normalizeDetailQuery("TaUrI")).toBe("tauri");
  });

  it("preserves internal whitespace because it can be part of a real match", () => {
    // "pnpm tauri dev" is a legitimate multi-word needle — only the
    // leading/trailing whitespace is noise.
    expect(normalizeDetailQuery("  pnpm tauri dev  ")).toBe("pnpm tauri dev");
  });
});
