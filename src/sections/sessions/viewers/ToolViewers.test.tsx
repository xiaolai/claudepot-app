import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

import type { LinkedTool } from "../../../types";
import { ToolExecutionView } from "./index";
import { computeDiff } from "./toolInput";

function mk(overrides: Partial<LinkedTool>): LinkedTool {
  return {
    tool_use_id: "toolu_test01",
    tool_name: "Read",
    model: null,
    call_ts: null,
    input_preview: "{}",
    result_ts: null,
    result_content: null,
    is_error: false,
    duration_ms: null,
    call_index: 0,
    result_index: null,
    ...overrides,
  };
}

describe("Edit viewer", () => {
  it("renders an inline diff for add + remove lines", () => {
    const tool = mk({
      tool_name: "Edit",
      input_preview: JSON.stringify({
        file_path: "/repo/src/lib.rs",
        old_string: "let x = 1;",
        new_string: "let x = 2;",
      }),
    });
    render(<ToolExecutionView tool={tool} />);
    expect(screen.getByTestId("edit-tool-viewer")).toBeInTheDocument();
    expect(screen.getByText("/repo/src/lib.rs")).toBeInTheDocument();
    expect(screen.getByText(/- let x = 1;/)).toBeInTheDocument();
    expect(screen.getByText(/\+ let x = 2;/)).toBeInTheDocument();
  });

  it("surfaces replace_all badge", () => {
    const tool = mk({
      tool_name: "Edit",
      input_preview: JSON.stringify({
        file_path: "/a",
        old_string: "a",
        new_string: "b",
        replace_all: true,
      }),
    });
    render(<ToolExecutionView tool={tool} />);
    expect(screen.getByText("replace all")).toBeInTheDocument();
  });

  it("falls back to raw dump when preview is clipped mid-JSON", () => {
    const tool = mk({
      tool_name: "Edit",
      input_preview: '{"file_path":"/a","old',
    });
    render(<ToolExecutionView tool={tool} />);
    expect(screen.getByTestId("edit-tool-viewer-fallback")).toBeInTheDocument();
  });
});

describe("Read viewer", () => {
  it("honors embedded line numbers from CC", () => {
    // Live CC result: "<line>\t<content>" on each line. Numbering
    // should reflect the embedded values, not an arbitrary count.
    const tool = mk({
      tool_name: "Read",
      input_preview: JSON.stringify({
        file_path: "/repo/main.rs",
        offset: 10,
      }),
      result_content: "11\tfn main() {}\n12\tprintln!()",
    });
    render(<ToolExecutionView tool={tool} />);
    expect(screen.getByTestId("read-tool-viewer")).toBeInTheDocument();
    expect(screen.getByText("/repo/main.rs")).toBeInTheDocument();
    expect(screen.getByText("fn main() {}")).toBeInTheDocument();
    // Embedded numbers surface, not 1/2 from the front.
    expect(screen.getByText("11")).toBeInTheDocument();
    expect(screen.getByText("12")).toBeInTheDocument();
  });

  it("falls back to offset-based numbering for unnumbered bodies", () => {
    // Plain text body (no tab prefixes). The viewer should start
    // numbering from offset + 1 instead of pretending it's line 1.
    const tool = mk({
      tool_name: "Read",
      input_preview: JSON.stringify({
        file_path: "/no-numbers.txt",
        offset: 99,
      }),
      result_content: "first\nsecond",
    });
    render(<ToolExecutionView tool={tool} />);
    expect(screen.getByText("100")).toBeInTheDocument();
    expect(screen.getByText("101")).toBeInTheDocument();
    expect(screen.getByText("first")).toBeInTheDocument();
  });

  it("shows a placeholder when there's no result yet", () => {
    const tool = mk({
      tool_name: "Read",
      input_preview: JSON.stringify({ file_path: "/a.rs" }),
      result_content: null,
    });
    render(<ToolExecutionView tool={tool} />);
    expect(screen.getByText("(no result yet)")).toBeInTheDocument();
  });
});

describe("Write viewer", () => {
  it("renders content and char count", () => {
    const tool = mk({
      tool_name: "Write",
      input_preview: JSON.stringify({
        file_path: "/tmp/foo.txt",
        content: "hello world",
      }),
    });
    render(<ToolExecutionView tool={tool} />);
    expect(screen.getByTestId("write-tool-viewer")).toBeInTheDocument();
    expect(screen.getByText("/tmp/foo.txt")).toBeInTheDocument();
    expect(screen.getByText("11 chars")).toBeInTheDocument();
    expect(screen.getByText("hello world")).toBeInTheDocument();
  });
});

describe("Bash viewer", () => {
  it("splits stdout and stderr when the result is JSON (cmd field)", () => {
    const tool = mk({
      tool_name: "Bash",
      input_preview: JSON.stringify({
        // Live CC transcripts use `cmd`.
        cmd: "cargo test",
        description: "run tests",
      }),
      result_content: JSON.stringify({
        stdout: "ok 1 test",
        stderr: "warning: unused",
        exit_code: 0,
      }),
    });
    render(<ToolExecutionView tool={tool} />);
    expect(screen.getByText("$ cargo test")).toBeInTheDocument();
    expect(screen.getByText("stdout")).toBeInTheDocument();
    expect(screen.getByText("stderr")).toBeInTheDocument();
    expect(screen.getByText(/ok 1 test/)).toBeInTheDocument();
    expect(screen.getByText(/warning: unused/)).toBeInTheDocument();
    expect(screen.getByText(/exit 0/i)).toBeInTheDocument();
  });

  it("falls back to legacy `command` field when `cmd` is absent", () => {
    const tool = mk({
      tool_name: "Bash",
      input_preview: JSON.stringify({ command: "ls -la" }),
      result_content: "a\nb",
    });
    render(<ToolExecutionView tool={tool} />);
    expect(screen.getByText("$ ls -la")).toBeInTheDocument();
  });

  it("treats plain-text result as stdout", () => {
    const tool = mk({
      tool_name: "Bash",
      input_preview: JSON.stringify({ cmd: "echo hi" }),
      result_content: "hi",
    });
    render(<ToolExecutionView tool={tool} />);
    expect(screen.getByText("hi")).toBeInTheDocument();
    expect(screen.queryByText("stderr")).not.toBeInTheDocument();
  });
});

describe("Generic fallback", () => {
  it("renders for unknown tool names", () => {
    const tool = mk({ tool_name: "SomeFutureTool" });
    render(<ToolExecutionView tool={tool} />);
    expect(screen.getByTestId("tool-viewer-somefuturetool")).toBeInTheDocument();
  });

  it("surfaces 'orphan' badge when there's no result", () => {
    const tool = mk({
      tool_name: "FutureTool",
      result_content: null,
    });
    render(<ToolExecutionView tool={tool} />);
    expect(screen.getByText("orphan")).toBeInTheDocument();
  });
});

describe("computeDiff", () => {
  it("drops unchanged prefix and keeps contextual neighbors", () => {
    const d = computeDiff("one\ntwo\nthree", "one\nTWO\nthree");
    // Expect one context line (one), removed (two), added (TWO), context (three).
    expect(d.map((x) => x.kind)).toEqual([
      "context",
      "remove",
      "add",
      "context",
    ]);
  });

  it("handles pure addition (old_string empty)", () => {
    const d = computeDiff("", "new line");
    expect(d[0].kind).toBe("add");
    expect(d[0].text).toBe("new line");
  });

  it("handles pure removal (new_string empty)", () => {
    const d = computeDiff("gone", "");
    expect(d[0].kind).toBe("remove");
  });
});
