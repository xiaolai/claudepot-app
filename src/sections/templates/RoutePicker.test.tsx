import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { RoutePicker } from "./RoutePicker";
import type { TemplateRouteSummaryDto } from "../../types";

function r(overrides: Partial<TemplateRouteSummaryDto>): TemplateRouteSummaryDto {
  return {
    id: "r1",
    name: "OpenRouter",
    provider: "openrouter",
    model: "claude-sonnet-4-5",
    is_local: false,
    is_private_cloud: false,
    is_capable: true,
    ineligibility_reason: "",
    ...overrides,
  };
}

describe("RoutePicker — capability-filtered route surface", () => {
  it("renders nothing when 0 capable routes exist (template runs on default claude)", () => {
    const onChange = vi.fn();
    const { container } = render(
      <RoutePicker
        routes={[]}
        selectedRouteId={null}
        onChange={onChange}
        privacyClass="any"
        onOpenThirdParties={() => {}}
      />,
    );
    expect(container.firstChild).toBeNull();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("renders nothing and silently auto-selects when exactly one capable route exists", async () => {
    const onChange = vi.fn();
    const route = r({ id: "only", name: "Local LM", is_local: true });
    const { container } = render(
      <RoutePicker
        routes={[route]}
        selectedRouteId={null}
        onChange={onChange}
        privacyClass="any"
        onOpenThirdParties={() => {}}
      />,
    );
    expect(container.firstChild).toBeNull();
    await waitFor(() => expect(onChange).toHaveBeenCalledWith("only"));
  });

  it("renders a dropdown when more than one capable route exists", () => {
    render(
      <RoutePicker
        routes={[
          r({ id: "a", name: "Alpha" }),
          r({ id: "b", name: "Beta" }),
        ]}
        selectedRouteId={null}
        onChange={vi.fn()}
        privacyClass="any"
        onOpenThirdParties={() => {}}
      />,
    );
    expect(screen.getByRole("combobox")).toBeInTheDocument();
    expect(screen.getByText(/Default — claude/)).toBeInTheDocument();
    expect(
      screen.getByRole("option", { name: /Alpha · claude-sonnet-4-5/ }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("option", { name: /Beta · claude-sonnet-4-5/ }),
    ).toBeInTheDocument();
  });

  it("does not auto-select when more than one capable route exists", () => {
    const onChange = vi.fn();
    render(
      <RoutePicker
        routes={[r({ id: "a" }), r({ id: "b" })]}
        selectedRouteId={null}
        onChange={onChange}
        privacyClass="any"
        onOpenThirdParties={() => {}}
      />,
    );
    expect(onChange).not.toHaveBeenCalled();
  });

  it("does not re-auto-select when the user has already chosen a route", () => {
    const onChange = vi.fn();
    render(
      <RoutePicker
        routes={[r({ id: "only", is_local: true })]}
        selectedRouteId="only"
        onChange={onChange}
        privacyClass="any"
        onOpenThirdParties={() => {}}
      />,
    );
    // Already-selected → effect short-circuits, no onChange storm.
    expect(onChange).not.toHaveBeenCalled();
  });

  it("emits null when the user picks the default option", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <RoutePicker
        routes={[r({ id: "a" }), r({ id: "b" })]}
        selectedRouteId="a"
        onChange={onChange}
        privacyClass="any"
        onOpenThirdParties={() => {}}
      />,
    );
    await user.selectOptions(screen.getByRole("combobox"), "__default__");
    expect(onChange).toHaveBeenCalledWith(null);
  });

  it("groups ineligible routes under 'Not eligible' and disables them", () => {
    render(
      <RoutePicker
        routes={[
          r({ id: "ok", name: "Capable" }),
          r({ id: "bad-1", name: "Capable Two" }),
          r({
            id: "off",
            name: "Bad",
            is_capable: false,
            ineligibility_reason: "context too small",
          }),
        ]}
        selectedRouteId={null}
        onChange={vi.fn()}
        privacyClass="any"
        onOpenThirdParties={() => {}}
      />,
    );
    const ineligible = screen.getByRole("option", {
      name: /Bad · claude-sonnet-4-5 — context too small/,
    });
    expect(ineligible).toBeDisabled();
  });

  it("surfaces a deep-link to Third-parties when privacy=local and zero local routes are configured", async () => {
    const user = userEvent.setup();
    const onOpen = vi.fn();
    render(
      <RoutePicker
        routes={[]}
        selectedRouteId={null}
        onChange={vi.fn()}
        privacyClass="local"
        onOpenThirdParties={onOpen}
      />,
    );
    await user.click(screen.getByRole("button", { name: /Set one up in Third-parties/ }));
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it("does NOT auto-select when the only capable route is paired with ineligible ones (the dropdown must render so the ineligible reasons are surfaced)", () => {
    const onChange = vi.fn();
    render(
      <RoutePicker
        routes={[
          r({ id: "ok", name: "Good" }),
          r({
            id: "bad",
            is_capable: false,
            ineligibility_reason: "no Bash",
          }),
        ]}
        selectedRouteId={null}
        onChange={onChange}
        privacyClass="any"
        onOpenThirdParties={() => {}}
      />,
    );
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByRole("combobox")).toBeInTheDocument();
  });
});
