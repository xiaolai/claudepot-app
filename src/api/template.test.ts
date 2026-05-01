import { describe, expect, it, vi, beforeEach } from "vitest";

// Capture invoke calls — the api wrappers are 1:1 over @tauri-apps/api
// invoke. The test pins (a) the command name and (b) the argument shape
// so the Rust side and the renderer keep agreeing on the wire contract.
const invokeSpy = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...a: unknown[]) => invokeSpy(...a),
}));

import { templateApi } from "./template";
import type { TemplateInstanceDto } from "../types";

describe("templateApi — Tauri command surface", () => {
  beforeEach(() => {
    invokeSpy.mockReset();
    invokeSpy.mockResolvedValue(undefined);
  });

  it("templatesList → 'templates_list' with no args", async () => {
    await templateApi.templatesList();
    expect(invokeSpy).toHaveBeenCalledWith("templates_list");
  });

  it("templatesGet → 'templates_get' with { id } (camelCase Rust convention)", async () => {
    await templateApi.templatesGet("it.morning-health-check");
    expect(invokeSpy).toHaveBeenCalledWith("templates_get", {
      id: "it.morning-health-check",
    });
  });

  it("templatesSampleReport → 'templates_sample_report' with { id }", async () => {
    await templateApi.templatesSampleReport("it.morning-health-check");
    expect(invokeSpy).toHaveBeenCalledWith("templates_sample_report", {
      id: "it.morning-health-check",
    });
  });

  it("templatesCapableRoutes → 'templates_capable_routes' with { id }", async () => {
    await templateApi.templatesCapableRoutes("it.morning-health-check");
    expect(invokeSpy).toHaveBeenCalledWith("templates_capable_routes", {
      id: "it.morning-health-check",
    });
  });

  it("templatesInstall → 'templates_install' with { instance } envelope", async () => {
    const instance: TemplateInstanceDto = {
      blueprint_id: "it.morning-health-check",
      blueprint_schema_version: 1,
      placeholder_values: {},
      schedule: { kind: "daily", time: "08:00" },
    };
    await templateApi.templatesInstall(instance);
    expect(invokeSpy).toHaveBeenCalledWith("templates_install", { instance });
  });
});
