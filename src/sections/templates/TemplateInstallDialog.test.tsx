import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { TemplateInstallDialog } from "./TemplateInstallDialog";

const getSpy = vi.fn();
const routesSpy = vi.fn();
const sampleSpy = vi.fn();

vi.mock("../../api", () => ({
  api: {
    templatesGet: (...a: unknown[]) => getSpy(...a),
    templatesCapableRoutes: (...a: unknown[]) => routesSpy(...a),
    templatesSampleReport: (...a: unknown[]) => sampleSpy(...a),
    templatesInstall: vi.fn(),
  },
}));

describe("TemplateInstallDialog — standalone wrapper", () => {
  beforeEach(() => {
    getSpy.mockReset();
    routesSpy.mockReset();
    sampleSpy.mockReset();
  });

  it("renders nothing when open=false", () => {
    render(
      <TemplateInstallDialog
        open={false}
        templateId="t.x"
        onClose={() => {}}
        onInstalled={() => {}}
        onError={() => {}}
        onOpenThirdParties={() => {}}
      />,
    );
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(getSpy).not.toHaveBeenCalled();
  });

  it("renders nothing when templateId is null (the dialog has nothing to fetch)", () => {
    render(
      <TemplateInstallDialog
        open={true}
        templateId={null}
        onClose={() => {}}
        onInstalled={() => {}}
        onError={() => {}}
        onOpenThirdParties={() => {}}
      />,
    );
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(getSpy).not.toHaveBeenCalled();
  });

  it("mounts a dialog and starts loading the template when open=true and templateId is set", () => {
    getSpy.mockReturnValue(new Promise(() => {})); // never resolves — keep in loading state
    routesSpy.mockReturnValue(new Promise(() => {}));
    render(
      <TemplateInstallDialog
        open={true}
        templateId="t.x"
        onClose={() => {}}
        onInstalled={() => {}}
        onError={() => {}}
        onOpenThirdParties={() => {}}
      />,
    );
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(getSpy).toHaveBeenCalledWith("t.x");
  });
});
