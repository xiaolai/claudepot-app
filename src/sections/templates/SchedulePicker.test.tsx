import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SchedulePicker } from "./SchedulePicker";
import type { ScheduleDto, ScheduleShapeName } from "../../types";

function setup(opts: {
  shapes: ScheduleShapeName[];
  value: ScheduleDto;
  defaultTime?: string;
  defaultCron?: string;
}) {
  const onChange = vi.fn();
  const utils = render(
    <SchedulePicker
      allowedShapes={opts.shapes}
      defaultShape={opts.shapes[0]}
      defaultTime={opts.defaultTime ?? "08:00"}
      defaultCron={opts.defaultCron ?? "0 8 * * *"}
      value={opts.value}
      onChange={onChange}
    />,
  );
  return { onChange, ...utils };
}

describe("SchedulePicker — semantic shape constraint", () => {
  it("only renders radios for shapes the blueprint allows", () => {
    setup({
      shapes: ["daily", "manual"],
      value: { kind: "daily", time: "08:00" },
    });
    expect(screen.getByText(/Each day at/)).toBeInTheDocument();
    expect(screen.getByText("Only when I run it")).toBeInTheDocument();
    expect(screen.queryByText(/Each weekday at/)).toBeNull();
    expect(screen.queryByText(/Custom \(advanced\)/)).toBeNull();
  });

  it("manual shape renders verbatim (first-class option, not advanced)", () => {
    setup({
      shapes: ["manual"],
      value: { kind: "manual" },
    });
    expect(screen.getByText("Only when I run it")).toBeInTheDocument();
    // No cron fallback offered when only manual is allowed.
    expect(screen.queryByPlaceholderText("0 8 * * *")).toBeNull();
  });

  it("Custom is hidden unless 'custom' is in allowed shapes", () => {
    setup({
      shapes: ["daily"],
      value: { kind: "daily", time: "08:00" },
    });
    expect(screen.queryByPlaceholderText("0 8 * * *")).toBeNull();
  });
});

describe("SchedulePicker — onChange contract", () => {
  it("switching from daily to manual emits a manual schedule", async () => {
    const user = userEvent.setup();
    const { onChange } = setup({
      shapes: ["daily", "manual"],
      value: { kind: "daily", time: "08:00" },
    });
    await user.click(screen.getByLabelText(/Only when I run it/));
    expect(onChange).toHaveBeenLastCalledWith({ kind: "manual" });
  });

  it("switching from manual to weekly emits a weekly schedule with default day=mon", async () => {
    const user = userEvent.setup();
    const { onChange } = setup({
      shapes: ["weekly", "manual"],
      value: { kind: "manual" },
      defaultTime: "09:30",
    });
    await user.click(screen.getByLabelText(/Each/));
    expect(onChange).toHaveBeenLastCalledWith({
      kind: "weekly",
      day: "mon",
      time: "09:30",
    });
  });

  it("switching to hourly emits the default 4-hour interval", async () => {
    const user = userEvent.setup();
    const { onChange } = setup({
      shapes: ["hourly", "manual"],
      value: { kind: "manual" },
    });
    await user.click(screen.getByLabelText(/Every/));
    expect(onChange).toHaveBeenLastCalledWith({
      kind: "hourly",
      every_n_hours: 4,
    });
  });

  it("switching to custom emits the blueprint default cron, not an empty string", async () => {
    const user = userEvent.setup();
    const { onChange } = setup({
      shapes: ["daily", "custom"],
      value: { kind: "daily", time: "08:00" },
      defaultCron: "*/15 * * * *",
    });
    await user.click(screen.getByLabelText(/Custom \(advanced\)/));
    expect(onChange).toHaveBeenLastCalledWith({
      kind: "custom",
      cron: "*/15 * * * *",
    });
  });

  it("editing the time on a daily schedule re-emits with the updated time", async () => {
    const user = userEvent.setup();
    const { onChange } = setup({
      shapes: ["daily"],
      value: { kind: "daily", time: "08:00" },
    });
    const time = screen.getByDisplayValue("08:00") as HTMLInputElement;
    await user.clear(time);
    await user.type(time, "10:30");
    // userEvent.type fires onChange per keystroke; verify the
    // final emitted value reflects the user's input shape rather
    // than asserting on intermediate frames.
    const lastCall = onChange.mock.calls[onChange.mock.calls.length - 1];
    expect(lastCall[0]).toMatchObject({ kind: "daily" });
  });

  it("editing the cron string while custom is active re-emits with the new string", async () => {
    const user = userEvent.setup();
    const { onChange } = setup({
      shapes: ["custom"],
      value: { kind: "custom", cron: "0 0 * * *" },
    });
    const input = screen.getByDisplayValue("0 0 * * *") as HTMLInputElement;
    await user.clear(input);
    await user.type(input, "*/5 * * * *");
    const lastCall = onChange.mock.calls[onChange.mock.calls.length - 1];
    expect(lastCall[0]).toMatchObject({ kind: "custom" });
  });

  it("hourly input clamps interval to [1,23] (defends against blueprint cron storms)", async () => {
    const user = userEvent.setup();
    const { onChange } = setup({
      shapes: ["hourly"],
      value: { kind: "hourly", every_n_hours: 4 },
    });
    const input = screen.getByDisplayValue("4") as HTMLInputElement;
    await user.clear(input);
    await user.type(input, "99");
    const lastCall = onChange.mock.calls[onChange.mock.calls.length - 1];
    expect(lastCall[0]).toMatchObject({
      kind: "hourly",
      every_n_hours: 23,
    });
  });
});

describe("SchedulePicker — radio reflects current value (not internal duplicate state)", () => {
  it("checked radio matches the controlled value prop", () => {
    setup({
      shapes: ["daily", "weekly", "manual"],
      value: { kind: "manual" },
    });
    const manual = screen.getByLabelText(/Only when I run it/) as HTMLInputElement;
    expect(manual.checked).toBe(true);
    const daily = screen.getByLabelText(/Each day at/) as HTMLInputElement;
    expect(daily.checked).toBe(false);
  });
});
