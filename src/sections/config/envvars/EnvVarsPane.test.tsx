import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const ccEnvListSpy = vi.fn();
const ccEnvSetSpy = vi.fn();
const ccEnvClearSpy = vi.fn();
vi.mock("../../../api", () => ({
  api: {
    ccEnvList: (...a: unknown[]) => ccEnvListSpy(...a),
    ccEnvSet: (...a: unknown[]) => ccEnvSetSpy(...a),
    ccEnvClear: (...a: unknown[]) => ccEnvClearSpy(...a),
  },
}));

import { EnvVarsPane } from "./EnvVarsPane";
import type {
  EnvOverview,
  EnvSafety,
  EnvValue,
  EnvVarSpec,
  EnvVarState,
} from "../../../types/ccEnv";

const safety = (over: Partial<EnvSafety> = {}): EnvSafety => ({
  secret: false,
  blocked_reason: null,
  pretrust_safe: true,
  provider_managed: false,
  hazards: [],
  ...over,
});

const spec = (over: Partial<EnvVarSpec>): EnvVarSpec => ({
  name: "X",
  category: "misc",
  doc: "docs prose",
  present_in_build: true,
  safety: safety(),
  control: "text",
  values: null,
  default: "",
  unit: "",
  on: null,
  off: null,
  numeric_evidence: "",
  format: "text",
  ...over,
});

const row = (s: EnvVarSpec, value: EnvValue = { state: "absent" }): EnvVarState => ({
  spec: s,
  settings_value: value,
  legacy_global: null,
  resolved_source: value.state === "absent" ? "no_known_file_override" : "settings_override",
});

const overview = (over: Partial<EnvOverview> = {}): EnvOverview => ({
  documented: [],
  undocumented: { state: "available", snapshot_version: "2.1.220", names: [] },
  unrecognized: [],
  docs_fetched_at: "2026-07-28",
  docs_sha256: "a".repeat(64),
  binary_crosscheck_version: "2.1.220",
  installed_version: "2.1.220",
  installed_path: "/opt/claude",
  settings_path: "/home/u/.claude/settings.json",
  categories: [
    { key: "auth", label: "Auth & Providers" },
    { key: "model", label: "Models" },
    { key: "limit", label: "Limits & Timeouts" },
    { key: "misc", label: "Other" },
  ],
  crosscheck_is_exact: true,
  ...over,
});

const TOGGLE2 = spec({ name: "DISABLE_COST_WARNINGS", control: "toggle", on: "1", off: "unset", values: ["1"] });
const TRISTATE = spec({ name: "USE_BUILTIN_RIPGREP", control: "toggle", on: "1", off: "0", values: ["0", "1"] });
const ENUMV = spec({ name: "CLAUDE_CODE_EFFORT_LEVEL", control: "enum", values: ["low", "high"] });
const NUMBER = spec({ name: "MAX_THINKING_TOKENS", control: "number", unit: "tokens", default: "31999", numeric_evidence: "name-suffix" });
const SECRET = spec({ name: "ANTHROPIC_API_KEY", format: "secret", safety: safety({ secret: true, pretrust_safe: false, hazards: ["switch_project"] }) });
const HAZARD = spec({ name: "ANTHROPIC_BASE_URL", format: "url", safety: safety({ pretrust_safe: false, hazards: ["redirect"] }) });
const BLOCKED = spec({ name: "CLAUDE_CONFIG_DIR", format: "path", safety: safety({ pretrust_safe: false, blocked_reason: "bootstrap_split_brain", hazards: ["unknown"] }) });

beforeEach(() => {
  ccEnvListSpy.mockReset();
  ccEnvSetSpy.mockReset();
  ccEnvClearSpy.mockReset();
});

describe("EnvVarsPane choosers", () => {
  it("renders the chooser each control type calls for", async () => {
    ccEnvListSpy.mockResolvedValue(
      overview({
        documented: [row(TOGGLE2), row(TRISTATE), row(ENUMV), row(NUMBER), row(SECRET)],
      }),
    );
    render(<EnvVarsPane />);

    // Two-state toggle → a switch.
    await waitFor(() =>
      expect(screen.getByRole("switch", { name: "DISABLE_COST_WARNINGS" })).toBeTruthy(),
    );
    // Three-state toggle → a radiogroup, not three loose buttons.
    const group = screen.getByRole("radiogroup", { name: /USE_BUILTIN_RIPGREP/ });
    expect(within(group).getAllByRole("radio")).toHaveLength(3);
    // Enum → a select whose first option removes the key.
    const select = screen.getByRole("combobox", { name: /CLAUDE_CODE_EFFORT_LEVEL/ });
    expect(within(select).getAllByRole("option")[0].textContent).toMatch(/unset/i);
    // Number → the documented default is a PLACEHOLDER, not a value.
    const num = screen.getByLabelText("MAX_THINKING_TOKENS value") as HTMLInputElement;
    expect(num.value).toBe("");
    expect(num.placeholder).toBe("31999");
    // Secret → a write-only password field plus a not-set badge.
    const sec = screen.getByLabelText("ANTHROPIC_API_KEY new value") as HTMLInputElement;
    expect(sec.type).toBe("password");
    expect(screen.getByText("not set")).toBeTruthy();
  });

  it("never renders a secret value, only whether it is set", async () => {
    ccEnvListSpy.mockResolvedValue(
      overview({ documented: [row(SECRET, { state: "secret_set" })] }),
    );
    const { container } = render(<EnvVarsPane />);
    await waitFor(() => expect(screen.getByText("set")).toBeTruthy());
    const field = screen.getByLabelText("ANTHROPIC_API_KEY new value") as HTMLInputElement;
    expect(field.value).toBe("");
    expect(container.textContent).not.toMatch(/sk-ant/);
  });

  /// Eleven documented variables use the `true`/`false` vocabulary and used
  /// to ship as free text — a boolean rendered as an open box. The segments
  /// show the variable's OWN literals: `1` where the doc says `1`, `true`
  /// where it says `true`.
  it("offers a boolean's documented literals rather than translating them", async () => {
    const BOOL_TRI = spec({
      name: "CLAUDE_CODE_AUTO_CONNECT_IDE",
      control: "toggle",
      on: "true",
      off: "false",
      values: ["false", "true"],
    });
    const BOOL_TWO = spec({
      name: "OTEL_METRICS_INCLUDE_VERSION",
      control: "toggle",
      on: "true",
      off: "unset",
      values: ["true"],
    });
    ccEnvListSpy.mockResolvedValue(
      overview({ documented: [row(BOOL_TRI), row(BOOL_TWO)] }),
    );
    // The write response must carry BOTH rows — it replaces the whole
    // overview, and dropping one would remove it from the list mid-test.
    ccEnvSetSpy.mockResolvedValue(
      overview({ documented: [row(BOOL_TRI), row(BOOL_TWO)] }),
    );
    render(<EnvVarsPane />);

    const group = await screen.findByRole("radiogroup", {
      name: /CLAUDE_CODE_AUTO_CONNECT_IDE/,
    });
    expect(
      within(group)
        .getAllByRole("radio")
        .map((r) => r.textContent),
    ).toEqual(["Unset", "false", "true"]);

    await userEvent.click(within(group).getByRole("radio", { name: "true" }));
    await waitFor(() =>
      expect(ccEnvSetSpy).toHaveBeenCalledWith(
        "CLAUDE_CODE_AUTO_CONNECT_IDE",
        "true",
      ),
    );

    // A documented-`true`-only variable stays two-state, and turning it on
    // writes `true` rather than `1`.
    const sw = screen.getByRole("switch", { name: "OTEL_METRICS_INCLUDE_VERSION" });
    await userEvent.click(sw);
    await waitFor(() =>
      expect(ccEnvSetSpy).toHaveBeenCalledWith(
        "OTEL_METRICS_INCLUDE_VERSION",
        "true",
      ),
    );
  });

  it("moves between tristate segments with the arrow keys", async () => {
    ccEnvListSpy.mockResolvedValue(overview({ documented: [row(TRISTATE)] }));
    ccEnvSetSpy.mockResolvedValue(overview({ documented: [row(TRISTATE)] }));
    render(<EnvVarsPane />);
    const group = await screen.findByRole("radiogroup", { name: /USE_BUILTIN_RIPGREP/ });
    const unset = within(group).getAllByRole("radio")[0];
    unset.focus();
    await userEvent.keyboard("{ArrowRight}");
    await waitFor(() => expect(ccEnvSetSpy).toHaveBeenCalledWith("USE_BUILTIN_RIPGREP", "0"));
  });
});

describe("EnvVarsPane commit semantics", () => {
  it("commits a safe text field on blur", async () => {
    const SAFE = spec({ name: "CLAUDE_CODE_SHELL_HINT", format: "text" });
    ccEnvListSpy.mockResolvedValue(overview({ documented: [row(SAFE)] }));
    ccEnvSetSpy.mockResolvedValue(overview({ documented: [row(SAFE)] }));
    render(<EnvVarsPane />);
    const input = await screen.findByLabelText("CLAUDE_CODE_SHELL_HINT value");
    await userEvent.type(input, "abc");
    await userEvent.tab();
    await waitFor(() =>
      expect(ccEnvSetSpy).toHaveBeenCalledWith("CLAUDE_CODE_SHELL_HINT", "abc"),
    );
  });

  it("blur alone never writes a hazardous value — Apply plus a confirmation does", async () => {
    ccEnvListSpy.mockResolvedValue(overview({ documented: [row(HAZARD)] }));
    ccEnvSetSpy.mockResolvedValue(overview({ documented: [row(HAZARD)] }));
    render(<EnvVarsPane />);
    const input = await screen.findByLabelText("ANTHROPIC_BASE_URL value");
    await userEvent.type(input, "https://evil.example");
    await userEvent.tab();
    expect(ccEnvSetSpy).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole("button", { name: "Apply" }));
    // Still not written — the confirmation names the risk first.
    expect(ccEnvSetSpy).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog").textContent).toMatch(/redirects Claude Code's traffic/);

    await userEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Apply" }));
    await waitFor(() =>
      expect(ccEnvSetSpy).toHaveBeenCalledWith("ANTHROPIC_BASE_URL", "https://evil.example"),
    );
  });

  it("a secret needs an explicit Store action and names the plaintext consequence", async () => {
    ccEnvListSpy.mockResolvedValue(overview({ documented: [row(SECRET)] }));
    ccEnvSetSpy.mockResolvedValue(overview({ documented: [row(SECRET)] }));
    render(<EnvVarsPane />);
    const field = await screen.findByLabelText("ANTHROPIC_API_KEY new value");
    await userEvent.type(field, "sk-ant-oat01-x");
    await userEvent.tab();
    expect(ccEnvSetSpy).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole("button", { name: /Store in settings.json/ }));
    const dialog = screen.getByRole("dialog");
    expect(dialog.textContent).toMatch(/plaintext/);
    // Secret AND hazardous → one combined confirmation, not two dialogs.
    expect(dialog.textContent).toMatch(/bill and log to/);
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
  });

  /**
   * The write is gated behind a confirmation, so the plaintext must not be
   * copied into React state to wait there. It lives in the password field's
   * DOM node until the moment of the write, and the field is blanked on
   * every outcome — including cancel.
   */
  it("keeps the pasted secret out of React state and wipes it on every outcome", async () => {
    ccEnvListSpy.mockResolvedValue(overview({ documented: [row(SECRET)] }));
    ccEnvSetSpy.mockResolvedValue(overview({ documented: [row(SECRET)] }));
    render(<EnvVarsPane />);
    const field = (await screen.findByLabelText(
      "ANTHROPIC_API_KEY new value",
    )) as HTMLInputElement;

    // Cancel path: the paste is destroyed, not left sitting in the field.
    await userEvent.type(field, "sk-ant-oat01-cancelled");
    await userEvent.click(
      screen.getByRole("button", { name: /Store in settings.json/ }),
    );
    await userEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "Cancel" }),
    );
    expect(ccEnvSetSpy).not.toHaveBeenCalled();
    expect(field.value).toBe("");

    // Confirm path: the value reaches the bridge exactly once, and the field
    // is blank again straight after. Re-query — the row re-rendered.
    const field2 = screen.getByLabelText(
      "ANTHROPIC_API_KEY new value",
    ) as HTMLInputElement;
    await userEvent.type(field2, "sk-ant-oat01-stored");
    // The field is uncontrolled, so the DOM node holds the whole value —
    // a controlled field would have reset it on the first keystroke.
    expect(field2.value).toBe("sk-ant-oat01-stored");
    await userEvent.click(
      screen.getByRole("button", { name: /Store in settings.json/ }),
    );
    await userEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "Apply" }),
    );
    await waitFor(() =>
      expect(ccEnvSetSpy).toHaveBeenCalledWith(
        "ANTHROPIC_API_KEY",
        "sk-ant-oat01-stored",
      ),
    );
    expect(ccEnvSetSpy).toHaveBeenCalledTimes(1);
    expect(
      (screen.getByLabelText("ANTHROPIC_API_KEY new value") as HTMLInputElement)
        .value,
    ).toBe("");
  });
});

describe("EnvVarsPane restore-default semantics", () => {
  it("Clear issues cc_env_clear, never a set of the documented default", async () => {
    ccEnvListSpy.mockResolvedValue(
      overview({ documented: [row(NUMBER, { state: "known", value: "100" })] }),
    );
    ccEnvClearSpy.mockResolvedValue(overview({ documented: [row(NUMBER)] }));
    render(<EnvVarsPane />);
    await userEvent.click(await screen.findByRole("button", { name: "Clear" }));
    await userEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "Remove" }),
    );
    await waitFor(() =>
      expect(ccEnvClearSpy).toHaveBeenCalledWith("MAX_THINKING_TOKENS"),
    );
    expect(ccEnvSetSpy).not.toHaveBeenCalled();
  });

  it("warns that clearing lets a lower source surface, and that running sessions keep the old value", async () => {
    const withLegacy: EnvVarState = {
      spec: NUMBER,
      settings_value: { state: "known", value: "100" },
      legacy_global: { state: "known", value: "50" },
      resolved_source: "settings_override",
    };
    ccEnvListSpy.mockResolvedValue(overview({ documented: [withLegacy] }));
    render(<EnvVarsPane />);
    expect(
      await screen.findByText(/Clearing here lets that value surface/),
    ).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Clear" }));
    expect(screen.getByRole("dialog").textContent).toMatch(
      /keep the old value until you relaunch/,
    );
  });
});

describe("EnvVarsPane read-only surfaces", () => {
  it("a blocked variable gets no input and states the reason inline", async () => {
    ccEnvListSpy.mockResolvedValue({ ...overview({ documented: [row(BLOCKED)] }) });
    const { container } = render(<EnvVarsPane />);
    await waitFor(() => expect(screen.getByText("read-only")).toBeTruthy());
    expect(container.querySelectorAll("input")).toHaveLength(1); // search only
    expect(screen.getByText(/Set it in your shell instead/)).toBeTruthy();
  });

  it("a Custom value renders read-only with Replace and Clear, never coerced into the chooser", async () => {
    ccEnvListSpy.mockResolvedValue(
      overview({
        documented: [row(TRISTATE, { state: "custom", raw: "true", kind: "string" })],
      }),
    );
    render(<EnvVarsPane />);
    expect(await screen.findByText("true")).toBeTruthy();
    expect(screen.queryByRole("radiogroup")).toBeNull();
    expect(screen.getByRole("button", { name: "Replace" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Clear" })).toBeTruthy();
  });

  /// Replace must never write the documented default — it clears the key and
  /// hands the row back to its normal chooser. An earlier version sent
  /// `spec.default || ""`, which both violated the display-only rule and, for
  /// the many specs with no documented default, sent `""` into a number or
  /// enum the backend rejects.
  it("Replace clears the key and never writes a documented default", async () => {
    for (const s of [NUMBER, ENUMV, TRISTATE]) {
      ccEnvSetSpy.mockReset();
      ccEnvClearSpy.mockReset();
      ccEnvListSpy.mockResolvedValue(
        overview({
          documented: [row(s, { state: "custom", raw: "junk", kind: "string" })],
        }),
      );
      ccEnvClearSpy.mockResolvedValue(overview({ documented: [row(s)] }));
      const { unmount } = render(<EnvVarsPane />);

      await userEvent.click(
        await screen.findByRole("button", { name: "Replace" }),
      );
      await userEvent.click(
        within(screen.getByRole("dialog")).getByRole("button", { name: "Remove" }),
      );
      await waitFor(() => expect(ccEnvClearSpy).toHaveBeenCalledWith(s.name));
      expect(ccEnvSetSpy).not.toHaveBeenCalled();
      unmount();
    }
  });

  /// Core withholds on *content* as well as classification, so `secret_set`
  /// can arrive for a variable whose spec says `secret: false`. Keying the
  /// control on the spec flag alone dropped those into the ordinary chooser,
  /// which then showed an empty editable field for a variable that is set.
  it("treats a withheld value as withheld even when the spec is not marked secret", async () => {
    ccEnvListSpy.mockResolvedValue(
      overview({ documented: [row(HAZARD, { state: "secret_set" })] }),
    );
    render(<EnvVarsPane />);
    // A write-only field and a "set" badge, not a blank text input.
    expect(
      await screen.findByLabelText("ANTHROPIC_BASE_URL new value"),
    ).toHaveAttribute("type", "password");
    expect(screen.getByText("set")).toBeTruthy();
    expect(screen.queryByLabelText("ANTHROPIC_BASE_URL value")).toBeNull();
  });

  it("names the empty string rather than rendering nothing", async () => {
    ccEnvListSpy.mockResolvedValue(
      overview({
        documented: [row(HAZARD, { state: "custom", raw: "", kind: "string" })],
      }),
    );
    render(<EnvVarsPane />);
    // Empty string is a real, distinct process state; showing an empty node
    // would make it indistinguishable from unset.
    expect(await screen.findByText("(empty string)")).toBeTruthy();
  });

  it("a CustomOpaque value reports its shape and withholds its contents", async () => {
    ccEnvListSpy.mockResolvedValue(
      overview({
        documented: [row(HAZARD, { state: "custom_opaque", kind: "object" })],
      }),
    );
    const { container } = render(<EnvVarsPane />);
    expect(await screen.findByText(/a JSON object — contents not shown/)).toBeTruthy();
    expect(container.textContent).not.toMatch(/Bearer/);
  });
});

describe("EnvVarsPane buckets", () => {
  /**
   * Structural regression sentinel — NOT a layout guard.
   *
   * jsdom has no layout engine, so nothing here can observe that an element
   * is zero pixels tall. This assertion can pass while the pane is visually
   * broken, and it is only meaningful alongside
   * `scripts/check-envvar-layout.mjs`, which measures real geometry in a
   * browser. What it pins is the one structural fact that browser check
   * cannot: where the appendix sits in the DOM.
   *
   * The pane shipped with both buckets as siblings of `.envvar-list`. That
   * put inflexible 3000px boxes next to a `flex: 1 1 0%` scroll container,
   * which pinned the list at 0px — every documented row was in the DOM and
   * none was on screen. Moving them back out reintroduces exactly that.
   */
  it("keeps both appendix buckets inside the scroll container", async () => {
    ccEnvListSpy.mockResolvedValue(
      overview({
        unrecognized: [
          { name: "SOME_HAND_SET_KEY", value: { state: "withheld", kind: "string" } },
        ],
        undocumented: {
          state: "available",
          snapshot_version: "2.1.220",
          names: ["ANTHROPIC_CONFIG_DIR"],
        },
      }),
    );
    const { container } = render(<EnvVarsPane />);
    await screen.findByText("ANTHROPIC_CONFIG_DIR");

    const list = container.querySelector(".envvar-list");
    expect(list, ".envvar-list must exist — it is the pane's scroll container")
      .toBeTruthy();

    const buckets = container.querySelectorAll(".envvar-bucket");
    expect(buckets.length).toBeGreaterThan(0);
    for (const bucket of buckets) {
      expect(
        list!.contains(bucket),
        `an appendix bucket is outside .envvar-list — this is the shipped ` +
          `bug that pinned the list to 0px; see scripts/check-envvar-layout.mjs`,
      ).toBe(true);
    }
  });

  /**
   * The chips look identical but combine differently — Type is an OR (a
   * variable is exactly one type), Attributes an AND (a variable can be
   * several at once). They shipped as one undifferentiated row of eight,
   * which gave the user no way to know that. The grouping and the rule text
   * ARE the signal, so flattening them back is a regression even though the
   * filtering behaviour would be unchanged.
   */
  it("returns the scroll container to the top when the filters change", async () => {
    // The appendix buckets share this scroller and dwarf the results, so a
    // stale offset can leave every match rendered above the viewport while
    // the count says matches exist.
    ccEnvListSpy.mockResolvedValue(
      overview({ documented: [row(TOGGLE2), row(SECRET)] }),
    );
    const { container } = render(<EnvVarsPane />);
    await screen.findByText("DISABLE_COST_WARNINGS");

    const list = container.querySelector(".envvar-list") as HTMLElement;
    list.scrollTop = 400;

    const search = screen.getByLabelText("Search environment variables");
    await userEvent.type(search, "SECRET");

    await waitFor(() => expect(list.scrollTop).toBe(0));
  });

  it("separates the two filter facets and states how each combines", async () => {
    ccEnvListSpy.mockResolvedValue(overview({ documented: [row(TOGGLE2)] }));
    render(<EnvVarsPane />);
    await screen.findByText("DISABLE_COST_WARNINGS");

    const type = screen.getByRole("group", { name: /Type/ });
    const attrs = screen.getByRole("group", { name: /Attributes/ });
    expect(type).not.toBe(attrs);

    // Each facet holds only its own chips.
    expect(within(type).getByRole("switch", { name: "toggle" })).toBeTruthy();
    expect(within(type).queryByRole("switch", { name: "secret" })).toBeNull();
    expect(within(attrs).getByRole("switch", { name: "secret" })).toBeTruthy();
    expect(within(attrs).queryByRole("switch", { name: "toggle" })).toBeNull();

    // The combination rule is stated, not left to be inferred.
    expect(type.getAttribute("aria-labelledby")).toBeTruthy();
    expect(screen.getByText(/— any/)).toBeVisible();
    expect(screen.getByText(/— all/)).toBeVisible();
  });

  it("exposes the scroll container as a named, keyboard-reachable region", async () => {
    ccEnvListSpy.mockResolvedValue(overview({ documented: [row(TOGGLE2)] }));
    render(<EnvVarsPane />);
    await screen.findByText("DISABLE_COST_WARNINGS");

    const region = screen.getByRole("region", {
      name: "Environment variable results",
    });
    // A scrollable box that cannot take focus cannot be scrolled by keyboard.
    expect(region).toHaveAttribute("tabindex", "0");
  });

  it("says so when a filter matches nothing, rather than going blank", async () => {
    ccEnvListSpy.mockResolvedValue(overview({ documented: [row(TOGGLE2)] }));
    render(<EnvVarsPane />);
    const search = await screen.findByLabelText("Search environment variables");
    await userEvent.type(search, "zzzznotathing");

    expect(
      await screen.findByText(/No documented variable matches these filters/i),
    ).toBeTruthy();
    expect(screen.getByText("0 of 1 documented variables")).toBeTruthy();
  });

  it("the undocumented bucket renders names but zero inputs on an exact match", async () => {
    ccEnvListSpy.mockResolvedValue(
      overview({
        undocumented: {
          state: "available",
          snapshot_version: "2.1.220",
          names: ["ANTHROPIC_CONFIG_DIR", "CLAUDE_CODE_SECRET_SAUCE"],
        },
      }),
    );
    const { container } = render(<EnvVarsPane />);
    expect(await screen.findByText("ANTHROPIC_CONFIG_DIR")).toBeTruthy();
    // Search field only — no control is offered for these.
    expect(container.querySelectorAll("input")).toHaveLength(1);
    expect(container.querySelectorAll("select")).toHaveLength(0);
    expect(screen.getByText(/edit/i).textContent).toBeTruthy();
  });

  it("collapses the undocumented names but never the explanation", async () => {
    ccEnvListSpy.mockResolvedValue(
      overview({
        undocumented: {
          state: "available",
          snapshot_version: "2.1.220",
          names: ["ANTHROPIC_CONFIG_DIR", "CLAUDE_CODE_SECRET_SAUCE"],
        },
      }),
    );
    render(<EnvVarsPane />);

    // Heading and explanation are always on screen — they carry the warning.
    expect(
      await screen.findByText(/Documented nowhere — found in Claude Code/),
    ).toBeVisible();
    expect(
      screen.getByText(/Claudepot deliberately\s+offers no control for them/),
    ).toBeVisible();

    // The 293-name dump does not lead. It sits behind a disclosure.
    const summary = screen.getByText(/Show 2 names/);
    expect(summary).toBeVisible();
    expect(screen.getByText("ANTHROPIC_CONFIG_DIR")).not.toBeVisible();

    await userEvent.click(summary);
    expect(screen.getByText("ANTHROPIC_CONFIG_DIR")).toBeVisible();
  });

  /**
   * AGENTS.md: on a snapshot/binary mismatch the section must render
   * "unavailable for this version". The disclosure added for the name list
   * must never wrap this branch — it is the one message the user needs, and
   * they have no reason to open a control to find it.
   */
  it("never hides the version-mismatch notice behind a disclosure", async () => {
    ccEnvListSpy.mockResolvedValue(
      overview({
        crosscheck_is_exact: false,
        installed_version: "2.2.0",
        undocumented: {
          state: "unavailable",
          snapshot_version: "2.1.220",
          installed_version: "2.2.0",
        },
      }),
    );
    const { container } = render(<EnvVarsPane />);

    expect(await screen.findByText(/Unavailable/)).toBeVisible();
    expect(container.querySelector("details")).toBeNull();
  });

  it("renders the unavailable state on a version mismatch, never stale names", async () => {
    ccEnvListSpy.mockResolvedValue(
      overview({
        crosscheck_is_exact: false,
        installed_version: "2.2.0",
        undocumented: {
          state: "unavailable",
          snapshot_version: "2.1.220",
          installed_version: "2.2.0",
        },
      }),
    );
    const { container } = render(<EnvVarsPane />);
    expect(await screen.findByText(/Unavailable/)).toBeTruthy();
    expect(container.textContent).toMatch(/2\.1\.220/);
    expect(container.textContent).toMatch(/2\.2\.0/);
    expect(container.textContent).not.toMatch(/ANTHROPIC_CONFIG_DIR/);
  });

  it("shows a hand-set unknown key as set, without its value", async () => {
    ccEnvListSpy.mockResolvedValue(
      overview({
        unrecognized: [
          { name: "TOTALLY_MADE_UP", value: { state: "withheld", kind: "string" } },
        ],
      }),
    );
    const { container } = render(<EnvVarsPane />);
    expect(await screen.findByText("TOTALLY_MADE_UP")).toBeTruthy();
    expect(screen.getByText(/set \(string\)/)).toBeTruthy();
    expect(container.textContent).not.toMatch(/could-be-a-token/);
    expect(screen.getByRole("button", { name: "Clear" })).toBeTruthy();
  });
});

describe("EnvVarsPane navigation and reconciliation", () => {
  it("clearing a hand-set unknown key sends that exact key", async () => {
    ccEnvListSpy.mockResolvedValue(
      overview({
        unrecognized: [
          { name: "TOTALLY_MADE_UP", value: { state: "withheld", kind: "string" } },
        ],
      }),
    );
    ccEnvClearSpy.mockResolvedValue(overview());
    render(<EnvVarsPane />);
    await userEvent.click(await screen.findByRole("button", { name: "Clear" }));
    await userEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "Remove" }),
    );
    await waitFor(() =>
      expect(ccEnvClearSpy).toHaveBeenCalledWith("TOTALLY_MADE_UP"),
    );
    expect(ccEnvSetSpy).not.toHaveBeenCalled();
  });

  it("points at the settings file it actually edits, not a hardcoded path", async () => {
    ccEnvListSpy.mockResolvedValue(
      overview({
        settings_path: "/custom/config/dir/settings.json",
        unrecognized: [
          { name: "SOMETHING", value: { state: "withheld", kind: "string" } },
        ],
        undocumented: {
          state: "available",
          snapshot_version: "2.1.220",
          names: ["CLAUDE_CODE_X"],
        },
      }),
    );
    const { container } = render(<EnvVarsPane />);
    await waitFor(() =>
      expect(
        screen.getAllByText("/custom/config/dir/settings.json").length,
      ).toBeGreaterThanOrEqual(2),
    );
    // CLAUDE_CONFIG_DIR moves the file; telling the user to edit a path we
    // are not writing would send them to the wrong file.
    expect(container.textContent).not.toMatch(/~\/\.claude\/settings\.json/);
  });

  it("does not claim undocumented names were found when there are none", async () => {
    ccEnvListSpy.mockResolvedValue(
      overview({
        undocumented: {
          state: "available",
          snapshot_version: "2.1.220",
          names: [],
        },
      }),
    );
    render(<EnvVarsPane />);
    expect(
      await screen.findByText(/Nothing undocumented was found/),
    ).toBeTruthy();
    expect(screen.queryByText(/were found by scanning/)).toBeNull();
  });

  it("filters by name and by doc text", async () => {
    ccEnvListSpy.mockResolvedValue(
      overview({
        documented: [
          row(spec({ name: "AAA_ONE", doc: "about kangaroos" })),
          row(spec({ name: "BBB_TWO", doc: "about wombats" })),
        ],
      }),
    );
    render(<EnvVarsPane />);
    const search = await screen.findByLabelText("Search environment variables");

    await userEvent.type(search, "AAA");
    await waitFor(() => expect(screen.queryByText("BBB_TWO")).toBeNull());
    expect(screen.getByText("AAA_ONE")).toBeTruthy();

    await userEvent.clear(search);
    await userEvent.type(search, "wombats");
    await waitFor(() => expect(screen.queryByText("AAA_ONE")).toBeNull());
    expect(screen.getByText("BBB_TWO")).toBeTruthy();
  });

  it("groups results by category rather than offering categories as the entry point", async () => {
    ccEnvListSpy.mockResolvedValue(
      overview({
        documented: [
          row(spec({ name: "MISC_ONE", category: "misc" })),
          row(spec({ name: "AUTH_ONE", category: "auth" })),
          row(spec({ name: "MISC_TWO", category: "misc" })),
        ],
      }),
    );
    render(<EnvVarsPane />);

    // Headings, in the generator's order — auth before misc, whatever
    // order the rows arrived in.
    const headings = await screen.findAllByRole("heading", { level: 4 });
    expect(headings.map((h) => h.textContent)).toEqual([
      "Auth & Providers 1",
      "Other 2",
    ]);

    // And a category is NOT a filter chip: `misc` alone holds 95 of the
    // 308 real variables, so chips over these would send a third of
    // every search into one undifferentiated bucket.
    expect(screen.queryByRole("button", { name: "Other" })).toBeNull();
  });

  /// A category the label table has never heard of still renders, under its
  /// raw key. Silently dropping the rows would be far worse than an ugly
  /// heading, and this is the failure mode the day the generator adds one.
  it("still renders rows whose category the label table does not know", async () => {
    ccEnvListSpy.mockResolvedValue(
      overview({
        documented: [
          row(spec({ name: "KNOWN_ONE", category: "auth" })),
          row(spec({ name: "FROM_THE_FUTURE", category: "quantum" })),
        ],
      }),
    );
    render(<EnvVarsPane />);
    expect(await screen.findByText("FROM_THE_FUTURE")).toBeTruthy();
    const headings = screen.getAllByRole("heading", { level: 4 });
    expect(headings.map((h) => h.textContent)).toEqual([
      "Auth & Providers 1",
      "quantum 1",
    ]);
  });

  it("keeps the grouping consistent with the active filter", async () => {
    ccEnvListSpy.mockResolvedValue(
      overview({
        documented: [
          row(spec({ name: "MISC_ONE", category: "misc" })),
          row(spec({ name: "AUTH_ONE", category: "auth" })),
        ],
      }),
    );
    render(<EnvVarsPane />);
    const search = await screen.findByLabelText("Search environment variables");
    await userEvent.type(search, "AUTH");

    await waitFor(() => {
      const headings = screen.getAllByRole("heading", { level: 4 });
      expect(headings.map((h) => h.textContent)).toEqual(["Auth & Providers 1"]);
    });
  });

  it("filters by control type and by safety attribute, and combines them", async () => {
    const setNumber = row(NUMBER, { state: "known", value: "1" });
    ccEnvListSpy.mockResolvedValue(
      overview({
        documented: [
          setNumber,
          row(TOGGLE2),
          row(SECRET),
          row(HAZARD),
          row(spec({ name: "PROVIDER_ONE", safety: safety({ provider_managed: true }) })),
        ],
      }),
    );
    render(<EnvVarsPane />);
    await screen.findByText("MAX_THINKING_TOKENS");

    // FilterChip renders as role="switch".
    const chip = (name: string) => screen.getByRole("switch", { name });
    const visible = () =>
      Array.from(document.querySelectorAll(".envvar-name")).map(
        (n) => n.textContent ?? "",
      );

    await userEvent.click(chip("toggle"));
    await waitFor(() => expect(visible()).toEqual(["DISABLE_COST_WARNINGS"]));
    await userEvent.click(chip("toggle"));

    await userEvent.click(chip("secret"));
    await waitFor(() => expect(visible()).toEqual(["ANTHROPIC_API_KEY"]));
    await userEvent.click(chip("secret"));

    await userEvent.click(chip("risky"));
    await waitFor(() =>
      expect(visible().sort()).toEqual(["ANTHROPIC_API_KEY", "ANTHROPIC_BASE_URL"]),
    );
    await userEvent.click(chip("risky"));

    await userEvent.click(chip("provider-managed"));
    await waitFor(() => expect(visible()).toEqual(["PROVIDER_ONE"]));
    await userEvent.click(chip("provider-managed"));

    await userEvent.click(chip("modified"));
    await waitFor(() => expect(visible()).toEqual(["MAX_THINKING_TOKENS"]));

    // Combining narrows: `modified` AND `secret` share no rows here.
    await userEvent.click(chip("secret"));
    await waitFor(() => expect(visible()).toEqual([]));
  });

  it("reconciles against the response rather than its own optimism", async () => {
    ccEnvListSpy.mockResolvedValue(overview({ documented: [row(ENUMV)] }));
    // The backend normalizes to something other than what was asked for.
    ccEnvSetSpy.mockResolvedValue(
      overview({ documented: [row(ENUMV, { state: "known", value: "high" })] }),
    );
    render(<EnvVarsPane />);
    const select = (await screen.findByRole("combobox", {
      name: /CLAUDE_CODE_EFFORT_LEVEL/,
    })) as HTMLSelectElement;
    await userEvent.selectOptions(select, "low");
    await waitFor(() => expect(select.value).toBe("high"));
  });

  it("re-reads from disk on refresh", async () => {
    ccEnvListSpy.mockResolvedValue(overview());
    render(<EnvVarsPane />);
    await waitFor(() => expect(ccEnvListSpy).toHaveBeenCalledTimes(1));
    await userEvent.click(screen.getByRole("button", { name: "Reload from disk" }));
    await waitFor(() => expect(ccEnvListSpy).toHaveBeenCalledTimes(2));
  });

  it("the (i) affordance opens a real disclosure, not a tooltip", async () => {
    ccEnvListSpy.mockResolvedValue(overview());
    render(<EnvVarsPane />);
    const info = await screen.findByRole("button", {
      name: "About environment variables",
    });
    await userEvent.click(info);
    expect(screen.getByText(/additively/)).toBeTruthy();
    expect(screen.getByText(/Unset is not zero/)).toBeTruthy();
    // Provenance is two facts, and the measured binary path is one of them.
    expect(screen.getByText("/opt/claude")).toBeTruthy();
    expect(screen.getByText(/2026-07-28/)).toBeTruthy();
  });
});
