import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import presetsFile from "../../../crates/claudepot-core/data/gateway-presets.json";
import { i18n } from "../../lib/i18n";
import { STORAGE_KEY_FROM_NETWORK_PANEL } from "../../lib/networkPanelDeepLink";

vi.mock("../../api", () => ({
  api: { routesDeriveSlug: async (m: string) => `claude-${m}` },
}));

import { RouteForm } from "./RouteForm";

const t = (k: string) => i18n.t(k as never, { ns: "providers" }) as string;

const preset = (id: string) => {
  const p = presetsFile.presets.find((x) => x.id === id);
  if (!p) throw new Error(`no preset ${id}`);
  return p;
};

function renderForm(mode: "add" | "edit" = "add") {
  render(<RouteForm mode={mode} onSubmit={async () => {}} onCancel={() => {}} />);
  return {
    baseUrl: () => screen.getByPlaceholderText("http://127.0.0.1:11434/v1"),
    model: () => screen.getByPlaceholderText(t("form.modelPlaceholderGateway")),
  };
}

beforeEach(() => sessionStorage.clear());
afterEach(() => sessionStorage.clear());

describe("RouteForm gateway presets", () => {
  it("fills the base URL and the preset's model", async () => {
    const f = renderForm();
    await userEvent.click(screen.getByRole("button", { name: "DeepSeek" }));
    expect(f.baseUrl()).toHaveValue(preset("deepseek").base_url);
    expect(f.model()).toHaveValue(preset("deepseek").model);
  });

  it("keeps the model the user typed when a local preset has none", async () => {
    const f = renderForm();
    await userEvent.type(f.model(), "qwen3:8b");
    await userEvent.click(screen.getByRole("button", { name: t("presets.ollamaLabel") }));
    expect(f.baseUrl()).toHaveValue(preset("ollama").base_url);
    expect(f.model()).toHaveValue("qwen3:8b");
  });

  it("replaces an earlier preset's URL with the next one clicked", async () => {
    const f = renderForm();
    await userEvent.click(screen.getByRole("button", { name: "DeepSeek" }));
    await userEvent.click(screen.getByRole("button", { name: t("presets.lmstudioLabel") }));
    expect(f.baseUrl()).toHaveValue(preset("lmstudio").base_url);
    // LM Studio carries no model, so DeepSeek's stays.
    expect(f.model()).toHaveValue(preset("deepseek").model);
  });

  it("offers every preset when opened normally", () => {
    renderForm();
    for (const name of ["DeepSeek", "Kimi (Moonshot)", "GLM (Z.ai)", "MiniMax"]) {
      expect(screen.getByRole("button", { name })).toBeInTheDocument();
    }
    expect(screen.getByText(t("presets.quickStartKnown"))).toBeInTheDocument();
  });

  it("offers only presets reachable from blocked regions when opened from the network panel", () => {
    sessionStorage.setItem(STORAGE_KEY_FROM_NETWORK_PANEL, "1");
    renderForm();
    expect(screen.getByText(t("presets.quickStartReachable"))).toBeInTheDocument();
    for (const p of presetsFile.presets) {
      const name = p.label ?? t(`presets.${p.id}Label`);
      const button = screen.queryByRole("button", { name });
      if (p.reachable_from_blocked_regions) expect(button, p.id).toBeInTheDocument();
      else expect(button, p.id).toBeNull();
    }
  });

  it("shows no presets when editing an existing route", () => {
    renderForm("edit");
    expect(screen.queryByRole("button", { name: "DeepSeek" })).toBeNull();
  });
});
