import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";

import { LiveStatusDot } from "./LiveStatusDot";

describe("LiveStatusDot", () => {
  it("renders a busy-tone dot keyed to --accent", () => {
    const { container } = render(<LiveStatusDot status="busy" title="Busy" />);
    const dot = container.querySelector("span");
    expect(dot).toBeTruthy();
    expect(dot!.getAttribute("title")).toBe("Busy");
    expect(dot!.style.background).toContain("--accent");
  });

  it("renders waiting tone via --warn", () => {
    const { container } = render(
      <LiveStatusDot status="waiting" title="Waiting" />,
    );
    expect(container.querySelector("span")!.style.background).toContain(
      "--warn",
    );
  });

  it("renders idle tone via --fg-muted", () => {
    const { container } = render(<LiveStatusDot status="idle" title="Idle" />);
    expect(container.querySelector("span")!.style.background).toContain(
      "--fg-muted",
    );
  });

  it("errored overlay overrides the base status with --danger", () => {
    const { container } = render(
      <LiveStatusDot status="busy" errored title="Errored" />,
    );
    expect(container.querySelector("span")!.style.background).toContain(
      "--danger",
    );
  });

  it("is aria-hidden by default (decorative)", () => {
    const { container } = render(<LiveStatusDot status="busy" />);
    const dot = container.querySelector("span")!;
    expect(dot.getAttribute("aria-hidden")).toBe("true");
    expect(dot.getAttribute("role")).toBeNull();
  });

  it("becomes role=img when aria-label is supplied", () => {
    const { container } = render(
      <LiveStatusDot status="busy" aria-label="Session busy" />,
    );
    const dot = container.querySelector("span")!;
    expect(dot.getAttribute("role")).toBe("img");
    expect(dot.getAttribute("aria-label")).toBe("Session busy");
    expect(dot.getAttribute("aria-hidden")).toBeNull();
  });
});
