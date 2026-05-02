import { describe, expect, it, vi, beforeEach } from "vitest";
import { useState } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TemplateGallery } from "./TemplateGallery";
import type { TemplateSummaryDto } from "../../types";

const listSpy = vi.fn();
const getSpy = vi.fn();
const routesSpy = vi.fn();
const installSpy = vi.fn();
const sampleSpy = vi.fn();

vi.mock("../../api", () => ({
  api: {
    templatesList: (...a: unknown[]) => listSpy(...a),
    templatesGet: (...a: unknown[]) => getSpy(...a),
    templatesCapableRoutes: (...a: unknown[]) => routesSpy(...a),
    templatesInstall: (...a: unknown[]) => installSpy(...a),
    templatesSampleReport: (...a: unknown[]) => sampleSpy(...a),
  },
}));

function tplSummary(overrides: Partial<TemplateSummaryDto>): TemplateSummaryDto {
  return {
    id: "t.id",
    name: "T name",
    tagline: "T tagline",
    category: "it-health",
    icon: "stethoscope",
    tier: "ambient",
    cost_class: "trivial",
    privacy: "any",
    recommended_class: "fast",
    consent_required: false,
    apply_supported: false,
    default_schedule_label: "Each day at 8 AM",
    ...overrides,
  };
}

function tplDetails(summary: TemplateSummaryDto) {
  return {
    summary,
    schema_version: 1,
    version: 1,
    description: `Description for ${summary.name}`,
    scope: { reads: "r", writes: "w", could_change: "c", network: "n" },
    capabilities_required: ["tool_use"],
    min_context_tokens: 4000,
    fallback_policy: "use_default_route",
    default_schedule_cron: "0 8 * * *",
    allowed_schedule_shapes: ["daily", "manual"],
    output_path_template: "reports/{{date}}.md",
    output_format: "markdown",
    placeholders: [],
    requires_full_disk_access: false,
  };
}

function setup(opts: {
  open?: boolean;
  templates?: TemplateSummaryDto[];
  onError?: (m: string) => void;
  onClose?: () => void;
  onInstalled?: () => void;
} = {}) {
  const templates = opts.templates ?? [
    tplSummary({ id: "it.morning-check", name: "Morning health check", category: "it-health" }),
    tplSummary({
      id: "audit.cache-cleanup",
      name: "Cache cleanup audit",
      category: "audit",
    }),
    tplSummary({
      id: "house.downloads",
      name: "Downloads tidy-up",
      category: "housekeeping",
    }),
  ];
  listSpy.mockResolvedValue(templates);
  routesSpy.mockResolvedValue([]);
  sampleSpy.mockResolvedValue("");

  const onClose = opts.onClose ?? vi.fn();
  const onError = opts.onError ?? vi.fn();
  const onInstalled = opts.onInstalled ?? vi.fn();
  const onOpenThirdParties = vi.fn();

  const utils = render(
    <TemplateGallery
      open={opts.open ?? true}
      onClose={onClose}
      onInstalled={onInstalled}
      onError={onError}
      onOpenThirdParties={onOpenThirdParties}
    />,
  );
  return { ...utils, onClose, onError, onInstalled, onOpenThirdParties };
}

describe("TemplateGallery — initial render", () => {
  beforeEach(() => {
    listSpy.mockReset();
    getSpy.mockReset();
    routesSpy.mockReset();
    installSpy.mockReset();
    sampleSpy.mockReset();
  });

  it("renders nothing when open=false (no fetch fires)", () => {
    listSpy.mockResolvedValue([]);
    render(
      <TemplateGallery
        open={false}
        onClose={() => {}}
        onInstalled={() => {}}
        onError={() => {}}
        onOpenThirdParties={() => {}}
      />,
    );
    expect(screen.queryByText("Install from template")).toBeNull();
    expect(listSpy).not.toHaveBeenCalled();
  });

  it("fetches the catalog when open transitions to true", async () => {
    setup();
    await waitFor(() => expect(listSpy).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("Install from template")).toBeInTheDocument();
  });

  it("renders one card per template surfaced by templates_list", async () => {
    setup();
    await screen.findByText("Morning health check");
    expect(screen.getByText("Cache cleanup audit")).toBeInTheDocument();
    expect(screen.getByText("Downloads tidy-up")).toBeInTheDocument();
  });

  it("renders category filter chips only when more than one category exists", async () => {
    setup({
      templates: [
        tplSummary({ id: "a", name: "Only IT", category: "it-health" }),
      ],
    });
    await screen.findByText("Only IT");
    // No filter chips when there is just one category.
    expect(screen.queryByRole("button", { name: "All" })).toBeNull();
  });
});

describe("TemplateGallery — category filtering", () => {
  beforeEach(() => {
    listSpy.mockReset();
    routesSpy.mockReset();
  });

  it("filters cards to the active category", async () => {
    const user = userEvent.setup();
    setup();
    await screen.findByText("Morning health check");
    await user.click(screen.getByRole("button", { name: "Audit" }));
    expect(screen.queryByText("Morning health check")).toBeNull();
    expect(screen.getByText("Cache cleanup audit")).toBeInTheDocument();
    expect(screen.queryByText("Downloads tidy-up")).toBeNull();
  });

  it("'All' returns to the unfiltered grid", async () => {
    const user = userEvent.setup();
    setup();
    await screen.findByText("Morning health check");
    await user.click(screen.getByRole("button", { name: "Audit" }));
    await user.click(screen.getByRole("button", { name: "All" }));
    expect(screen.getByText("Morning health check")).toBeInTheDocument();
    expect(screen.getByText("Cache cleanup audit")).toBeInTheDocument();
    expect(screen.getByText("Downloads tidy-up")).toBeInTheDocument();
  });

  it("renders 'No templates in this category' when the active filter has zero matches", async () => {
    const user = userEvent.setup();
    setup({
      templates: [tplSummary({ id: "a", name: "x", category: "it-health" }),
                  tplSummary({ id: "b", name: "y", category: "audit" })],
    });
    await screen.findByText("x");
    await user.click(screen.getByRole("button", { name: "Audit" }));
    expect(screen.queryByText("x")).toBeNull();
    expect(screen.getByText("y")).toBeInTheDocument();
  });
});

describe("TemplateGallery — single Modal swap to install view (regression: no backdrop flash)", () => {
  beforeEach(() => {
    listSpy.mockReset();
    getSpy.mockReset();
    routesSpy.mockReset();
    sampleSpy.mockReset();
  });

  it("clicking a card swaps the modal content from grid to install view in place", async () => {
    const user = userEvent.setup();
    const cacheTpl = tplSummary({
      id: "audit.cache-cleanup",
      name: "Cache cleanup audit",
      category: "audit",
    });
    listSpy.mockResolvedValue([cacheTpl]);
    getSpy.mockResolvedValue(tplDetails(cacheTpl));
    routesSpy.mockResolvedValue([]);
    sampleSpy.mockResolvedValue("");

    render(
      <TemplateGallery
        open
        onClose={() => {}}
        onInstalled={() => {}}
        onError={() => {}}
        onOpenThirdParties={() => {}}
      />,
    );

    await screen.findByText("Cache cleanup audit");
    // exactly one dialog throughout the swap — no second mount/unmount
    const dialogsBefore = screen.getAllByRole("dialog");
    expect(dialogsBefore).toHaveLength(1);

    await user.click(
      screen.getByRole("button", { name: /Cache cleanup audit/ }),
    );
    // The install view's own header replaces the grid header.
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "Cache cleanup audit", level: 2 }),
      ).toBeInTheDocument(),
    );
    expect(screen.queryByText("Install from template")).toBeNull();
    // Still exactly one dialog — content swapped, not remounted.
    const dialogsAfter = screen.getAllByRole("dialog");
    expect(dialogsAfter).toHaveLength(1);
    expect(dialogsAfter[0]).toBe(dialogsBefore[0]);
  });

  it("install view's Back button returns to the gallery grid (in-place swap, modal stays mounted)", async () => {
    const user = userEvent.setup();
    const tpl = tplSummary({ id: "t.x", name: "T X" });
    listSpy.mockResolvedValue([tpl]);
    getSpy.mockResolvedValue(tplDetails(tpl));
    routesSpy.mockResolvedValue([]);
    sampleSpy.mockResolvedValue("");

    render(
      <TemplateGallery
        open
        onClose={() => {}}
        onInstalled={() => {}}
        onError={() => {}}
        onOpenThirdParties={() => {}}
      />,
    );
    await screen.findByText("T X");
    await user.click(screen.getByRole("button", { name: /T X/ }));
    await screen.findByRole("heading", { name: "T X", level: 2 });

    await user.click(screen.getByRole("button", { name: "Back" }));
    expect(await screen.findByText("Install from template")).toBeInTheDocument();
  });

  it("Close from grid view triggers onClose; Close from install view does not (Back is the install-view affordance)", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    setup({ onClose });
    await screen.findByText("Morning health check");
    await user.click(screen.getByRole("button", { name: "Close" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

describe("TemplateGallery — onError ref-stash (regression: parent re-renders must not refire fetch)", () => {
  beforeEach(() => {
    listSpy.mockReset();
    getSpy.mockReset();
    routesSpy.mockReset();
    sampleSpy.mockReset();
  });

  it("does not re-fetch the catalog when the parent re-renders with a fresh onError lambda", async () => {
    const user = userEvent.setup();
    listSpy.mockResolvedValue([
      tplSummary({ id: "t.a", name: "A" }),
    ]);

    function Wrapper() {
      const [tick, setTick] = useState(0);
      // Inline lambda — fresh identity every render. The fix in
      // commit a730205 stashes onError in a ref so this DOES NOT
      // retrigger the fetch effect.
      return (
        <>
          <button data-testid="poke" onClick={() => setTick(tick + 1)}>
            poke
          </button>
          <TemplateGallery
            open
            onClose={() => {}}
            onInstalled={() => {}}
            onError={(_msg) => {
              void tick;
            }}
            onOpenThirdParties={() => {}}
          />
        </>
      );
    }
    render(<Wrapper />);
    await waitFor(() => expect(listSpy).toHaveBeenCalledTimes(1));

    await user.click(screen.getByTestId("poke"));
    await user.click(screen.getByTestId("poke"));
    await user.click(screen.getByTestId("poke"));

    expect(listSpy).toHaveBeenCalledTimes(1);
  });

  it("does not snap install→gallery on parent re-render (the regression that motivated the ref-stash)", async () => {
    const user = userEvent.setup();
    const tpl = tplSummary({ id: "t.x", name: "T X" });
    listSpy.mockResolvedValue([tpl]);
    getSpy.mockResolvedValue(tplDetails(tpl));
    routesSpy.mockResolvedValue([]);
    sampleSpy.mockResolvedValue("");

    function Wrapper() {
      const [tick, setTick] = useState(0);
      return (
        <>
          <button data-testid="poke" onClick={() => setTick(tick + 1)}>
            poke
          </button>
          <TemplateGallery
            open
            onClose={() => {}}
            onInstalled={() => {}}
            onError={() => {
              void tick;
            }}
            onOpenThirdParties={() => {}}
          />
        </>
      );
    }
    render(<Wrapper />);
    await screen.findByText("T X");
    await user.click(screen.getByRole("button", { name: /T X/ }));
    await screen.findByRole("heading", { name: "T X", level: 2 });

    // Bang on parent re-render mid-install. Pre-fix, this snapped
    // back to the gallery because the gallery's effect re-ran with
    // a fresh onError ref and reset installTarget=null.
    await user.click(screen.getByTestId("poke"));
    await user.click(screen.getByTestId("poke"));
    await user.click(screen.getByTestId("poke"));

    // Still on the install view.
    expect(
      screen.getByRole("heading", { name: "T X", level: 2 }),
    ).toBeInTheDocument();
    expect(screen.queryByText("Install from template")).toBeNull();
  });
});
