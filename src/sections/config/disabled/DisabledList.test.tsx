import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

vi.mock("../../../api", () => ({
  api: {
    artifactEnable: vi.fn(() => Promise.resolve()),
    artifactTrash: vi.fn(() => Promise.resolve()),
  },
}));

import { DisabledList } from "./DisabledList";
import type { DisabledRecordDto } from "../../../types";

function mkRecord(name: string): DisabledRecordDto {
  return {
    scope: "user",
    scope_root: "/home/u/.claude",
    kind: "skill",
    name,
    original_path: `/home/u/.claude/skills/${name}`,
    current_path: `/home/u/.claude/.disabled/skills/${name}`,
    payload_kind: "file",
  };
}

function renderList(
  rows: DisabledRecordDto[],
  overrides: Partial<Parameters<typeof DisabledList>[0]> = {},
) {
  const onSelect = vi.fn();
  render(
    <DisabledList
      rows={rows}
      selected={null}
      onSelect={onSelect}
      projectRoot={null}
      pushToast={vi.fn()}
      onChanged={vi.fn()}
      {...overrides}
    />,
  );
  return { onSelect };
}

describe("DisabledList", () => {
  it("renders rows as listbox options with aria-selected", () => {
    renderList([mkRecord("alpha"), mkRecord("beta")], {
      selected: mkRecord("beta"),
    });
    const list = screen.getByRole("listbox", { name: /disabled skills/i });
    const options = screen.getAllByRole("option");
    expect(list).toBeInTheDocument();
    expect(options).toHaveLength(2);
    expect(options[0]).toHaveAttribute("aria-selected", "false");
    expect(options[1]).toHaveAttribute("aria-selected", "true");
  });

  it("rows are keyboard-reachable and Enter selects (a11y floor)", async () => {
    const { onSelect } = renderList([mkRecord("alpha")]);
    const row = screen.getByRole("option");
    expect(row).toHaveAttribute("tabIndex", "0");

    row.focus();
    await userEvent.keyboard("{Enter}");
    expect(onSelect).toHaveBeenCalledTimes(1);

    await userEvent.keyboard(" ");
    expect(onSelect).toHaveBeenCalledTimes(2);
  });

  it("row actions: labeled Re-enable button + icon-only trash with aria-label", () => {
    renderList([mkRecord("alpha")]);
    // Tier 3 (icon-buttons.md): re-enable carries its verb as text.
    expect(
      screen.getByRole("button", { name: "Re-enable" }),
    ).toBeInTheDocument();
    // Tier 1: trash in a dense list is icon-only with aria-label.
    expect(
      screen.getByRole("button", { name: "Move to trash" }),
    ).toBeInTheDocument();
  });
});
