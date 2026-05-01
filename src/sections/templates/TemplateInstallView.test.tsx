import { describe, expect, it, vi, beforeEach } from "vitest";
import { useState } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TemplateInstallView } from "./TemplateInstallView";
import type {
  AutomationSummaryDto,
  TemplateDetailsDto,
  TemplateInstanceDto,
  TemplateRouteSummaryDto,
} from "../../types";

const getSpy = vi.fn();
const routesSpy = vi.fn();
const installSpy = vi.fn();
const sampleSpy = vi.fn();

vi.mock("../../api", () => ({
  api: {
    templatesGet: (...a: unknown[]) => getSpy(...a),
    templatesCapableRoutes: (...a: unknown[]) => routesSpy(...a),
    templatesInstall: (...a: unknown[]) => installSpy(...a),
    templatesSampleReport: (...a: unknown[]) => sampleSpy(...a),
  },
}));

function makeDetails(overrides: Partial<TemplateDetailsDto> = {}): TemplateDetailsDto {
  return {
    summary: {
      id: "it.morning-health-check",
      name: "Morning health check",
      tagline: "A quick read on how your Mac's doing.",
      category: "it-health",
      icon: "stethoscope",
      tier: "ambient",
      cost_class: "trivial",
      privacy: "any",
      recommended_class: "fast",
      consent_required: false,
      apply_supported: false,
      default_schedule_label: "Each morning at 8 AM",
      ...(overrides.summary ?? {}),
    },
    schema_version: 1,
    version: 1,
    description: "Reads disk free space and CPU pressure once a day.",
    scope: {
      reads: "Disk free space, CPU pressure.",
      writes: "A markdown file under ~/.claudepot/reports/.",
      could_change: "Nothing — read-only.",
      network: "None.",
    },
    capabilities_required: ["tool_use"],
    min_context_tokens: 4000,
    fallback_policy: "use_default_route",
    default_schedule_cron: "0 8 * * *",
    allowed_schedule_shapes: ["daily", "manual"],
    output_path_template: "reports/morning/{{date}}.md",
    output_format: "markdown",
    placeholders: [],
    requires_full_disk_access: false,
    ...overrides,
  };
}

function setup(opts: {
  templateId?: string;
  details?: Partial<TemplateDetailsDto>;
  routes?: TemplateRouteSummaryDto[];
  onError?: (m: string) => void;
  onInstalled?: () => void;
  onBack?: () => void;
} = {}) {
  getSpy.mockResolvedValue(makeDetails(opts.details ?? {}));
  routesSpy.mockResolvedValue(opts.routes ?? []);
  sampleSpy.mockResolvedValue("# Sample\n\nbody");

  const onError = opts.onError ?? vi.fn();
  const onInstalled = opts.onInstalled ?? vi.fn();
  const onBack = opts.onBack ?? vi.fn();
  const onOpenThirdParties = vi.fn();

  const utils = render(
    <TemplateInstallView
      templateId={opts.templateId ?? "it.morning-health-check"}
      onBack={onBack}
      onInstalled={onInstalled}
      onError={onError}
      onOpenThirdParties={onOpenThirdParties}
    />,
  );
  return { ...utils, onError, onInstalled, onBack, onOpenThirdParties };
}

describe("TemplateInstallView — initial load", () => {
  beforeEach(() => {
    getSpy.mockReset();
    routesSpy.mockReset();
    installSpy.mockReset();
    sampleSpy.mockReset();
  });

  it("renders the loading state until details + routes resolve", async () => {
    let resolveDetails!: (d: TemplateDetailsDto) => void;
    getSpy.mockReturnValue(
      new Promise<TemplateDetailsDto>((res) => {
        resolveDetails = res;
      }),
    );
    routesSpy.mockResolvedValue([]);
    sampleSpy.mockResolvedValue("");

    render(
      <TemplateInstallView
        templateId="t.x"
        onBack={() => {}}
        onInstalled={() => {}}
        onError={() => {}}
        onOpenThirdParties={() => {}}
      />,
    );
    expect(screen.getByText("Loading template…")).toBeInTheDocument();

    resolveDetails(makeDetails());
    await waitFor(() =>
      expect(screen.queryByText("Loading template…")).toBeNull(),
    );
    expect(screen.getByText("Morning health check")).toBeInTheDocument();
  });

  it("surfaces the description, tagline, and scope statements", async () => {
    setup();
    await screen.findByText("Morning health check");
    expect(
      screen.getByText("A quick read on how your Mac's doing."),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Reads disk free space and CPU pressure once a day."),
    ).toBeInTheDocument();
    expect(screen.getByText("Disk free space, CPU pressure.")).toBeInTheDocument();
  });

  it("starts with the sample report collapsed and toggles open on click", async () => {
    const user = userEvent.setup();
    setup();
    await screen.findByText("Morning health check");
    expect(screen.queryByText(/Loading sample/)).toBeNull();
    await user.click(screen.getByRole("button", { name: /View sample report/ }));
    // Sample loader fires now.
    await waitFor(() => expect(sampleSpy).toHaveBeenCalled());
  });
});

describe("TemplateInstallView — install action", () => {
  beforeEach(() => {
    getSpy.mockReset();
    routesSpy.mockReset();
    installSpy.mockReset();
    sampleSpy.mockReset();
  });

  it("posts a TemplateInstanceDto with blueprint id, version, default schedule, no route", async () => {
    const user = userEvent.setup();
    const onInstalled = vi.fn();
    installSpy.mockResolvedValue({} as AutomationSummaryDto);
    setup({ onInstalled });

    await screen.findByRole("button", { name: "Install" });
    await user.click(screen.getByRole("button", { name: "Install" }));

    await waitFor(() => expect(installSpy).toHaveBeenCalledTimes(1));
    const sent = installSpy.mock.calls[0][0] as TemplateInstanceDto;
    expect(sent.blueprint_id).toBe("it.morning-health-check");
    expect(sent.blueprint_schema_version).toBe(1);
    expect(sent.schedule).toEqual({ kind: "daily", time: "08:00" });
    expect(sent.route_id).toBeUndefined();
    expect(onInstalled).toHaveBeenCalledTimes(1);
  });

  it("disables Install + shows 'Installing…' while the request is in flight", async () => {
    const user = userEvent.setup();
    let resolveInstall!: (a: AutomationSummaryDto) => void;
    installSpy.mockReturnValue(
      new Promise<AutomationSummaryDto>((res) => {
        resolveInstall = res;
      }),
    );
    setup();
    await screen.findByRole("button", { name: "Install" });
    await user.click(screen.getByRole("button", { name: "Install" }));

    const busy = await screen.findByRole("button", { name: "Installing…" });
    expect(busy).toBeDisabled();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();

    resolveInstall({} as AutomationSummaryDto);
  });

  it("propagates an install error via onError without unmounting", async () => {
    const user = userEvent.setup();
    const onError = vi.fn();
    installSpy.mockRejectedValue(new Error("scheduler missing"));
    setup({ onError });
    await screen.findByRole("button", { name: "Install" });
    await user.click(screen.getByRole("button", { name: "Install" }));
    await waitFor(() => expect(onError).toHaveBeenCalled());
    const msg = String(onError.mock.calls[0][0]);
    expect(msg).toMatch(/scheduler missing/);
    // Button is back to the idle state — view still mounted.
    expect(screen.getByRole("button", { name: "Install" })).toBeEnabled();
  });

  it("disables Install when privacy=local and no local route is configured", async () => {
    setup({
      details: {
        summary: {
          id: "caregiver.heartbeat",
          name: "Caregiver heartbeat",
          tagline: "x",
          category: "caregiver",
          icon: "heart",
          tier: "ambient",
          cost_class: "trivial",
          privacy: "local",
          recommended_class: "local-ok",
          consent_required: true,
          apply_supported: false,
          default_schedule_label: "Each Saturday at 9 AM",
        },
        allowed_schedule_shapes: ["weekly", "manual"],
        default_schedule_cron: "0 9 * * 6",
      },
      routes: [
        // Capable but cloud → not allowed for privacy=local
        {
          id: "cloud",
          name: "Cloud",
          provider: "openrouter",
          model: "claude-sonnet-4-5",
          is_local: false,
          is_private_cloud: false,
          is_capable: true,
          ineligibility_reason: "",
        },
      ],
    });
    await screen.findByText("Caregiver heartbeat");
    expect(screen.getByRole("button", { name: "Install" })).toBeDisabled();
  });
});

describe("TemplateInstallView — onError ref-stash (regression: parent re-render must not refire effect)", () => {
  beforeEach(() => {
    getSpy.mockReset();
    routesSpy.mockReset();
    sampleSpy.mockReset();
    getSpy.mockResolvedValue(makeDetails());
    routesSpy.mockResolvedValue([]);
    sampleSpy.mockResolvedValue("");
  });

  it("does not re-fetch details when only the onError callback identity changes", async () => {
    function Wrapper() {
      const [tick, setTick] = useState(0);
      // Inline lambda — fresh identity every render. Mirrors the
      // `onError={(msg) => setToast(...)}` shape used by
      // AutomationsSection. The fix in `a730205` ref-stashes
      // onError so the fetch effect does NOT refire.
      return (
        <>
          <button data-testid="poke" onClick={() => setTick(tick + 1)}>
            poke
          </button>
          <TemplateInstallView
            templateId="t.x"
            onBack={() => {}}
            onInstalled={() => {}}
            onError={(msg) => {
              // identity changes every render
              void msg;
              void tick;
            }}
            onOpenThirdParties={() => {}}
          />
        </>
      );
    }
    const user = userEvent.setup();
    render(<Wrapper />);
    await waitFor(() => expect(getSpy).toHaveBeenCalledTimes(1));

    await user.click(screen.getByTestId("poke"));
    await user.click(screen.getByTestId("poke"));
    await user.click(screen.getByTestId("poke"));

    // Still one fetch — the ref-stash kept the effect from refiring.
    expect(getSpy).toHaveBeenCalledTimes(1);
  });
});

describe("TemplateInstallView — sticky action bar layout", () => {
  beforeEach(() => {
    getSpy.mockReset();
    routesSpy.mockReset();
    sampleSpy.mockReset();
    getSpy.mockResolvedValue(makeDetails());
    routesSpy.mockResolvedValue([]);
    sampleSpy.mockResolvedValue("");
  });

  it("renders Cancel + Install buttons (action bar present)", async () => {
    setup();
    await screen.findByRole("button", { name: "Install" });
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
  });

  it("backLabel prop overrides the Cancel label (gallery passes 'Back')", async () => {
    getSpy.mockResolvedValue(makeDetails());
    routesSpy.mockResolvedValue([]);
    sampleSpy.mockResolvedValue("");
    render(
      <TemplateInstallView
        templateId="t.x"
        onBack={() => {}}
        onInstalled={() => {}}
        onError={() => {}}
        onOpenThirdParties={() => {}}
        backLabel="Back"
      />,
    );
    await screen.findByRole("button", { name: "Install" });
    expect(screen.getByRole("button", { name: "Back" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull();
  });

  it("clicking Cancel calls onBack", async () => {
    const user = userEvent.setup();
    const onBack = vi.fn();
    setup({ onBack });
    await screen.findByRole("button", { name: "Install" });
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onBack).toHaveBeenCalledTimes(1);
  });
});
