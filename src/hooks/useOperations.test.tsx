import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { OperationsProvider, useOperations } from "./useOperations";

function Probe() {
  const { active, open, close } = useOperations();
  return (
    <div>
      <span data-testid="active">{active ? active.opId : "∅"}</span>
      <span data-testid="title">{active ? active.title : "∅"}</span>
      <button onClick={() => open({ opId: "op-a", title: "A" })}>open A</button>
      <button onClick={() => open({ opId: "op-b", title: "B" })}>open B</button>
      <button onClick={() => close()}>close</button>
    </div>
  );
}

describe("useOperations", () => {
  it("open sets the active op; close clears it", async () => {
    const user = userEvent.setup();
    render(
      <OperationsProvider>
        <Probe />
      </OperationsProvider>,
    );
    expect(screen.getByTestId("active")).toHaveTextContent("∅");

    await user.click(screen.getByText("open A"));
    expect(screen.getByTestId("active")).toHaveTextContent("op-a");
    expect(screen.getByTestId("title")).toHaveTextContent("A");

    await user.click(screen.getByText("close"));
    expect(screen.getByTestId("active")).toHaveTextContent("∅");
  });

  it("opening a second op replaces the first (prior op keeps running in strip)", async () => {
    const user = userEvent.setup();
    render(
      <OperationsProvider>
        <Probe />
      </OperationsProvider>,
    );
    await user.click(screen.getByText("open A"));
    expect(screen.getByTestId("active")).toHaveTextContent("op-a");

    await user.click(screen.getByText("open B"));
    expect(screen.getByTestId("active")).toHaveTextContent("op-b");
    // Prior modal is hidden; the prior op is NOT cancelled — that's the
    // RunningOpStrip's responsibility to keep visible.
  });

  it("throws if used outside the provider", () => {
    // Swallow the expected console.error from React.
    const err = console.error;
    console.error = () => {};
    try {
      expect(() => render(<Probe />)).toThrow(
        /useOperations must be used inside OperationsProvider/,
      );
    } finally {
      console.error = err;
    }
  });
});
