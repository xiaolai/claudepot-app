/**
 * Typed i18next keys — `t()` calls are compile-checked against the
 * English catalogs (the source of truth; zh-CN completeness is a test
 * concern, not a type concern). Adding a namespace means adding its
 * import here AND registering it in `src/lib/i18n.ts`.
 */
import "i18next";
import type accounts from "../locales/en/accounts.json";
import type activities from "../locales/en/activities.json";
import type agents from "../locales/en/agents.json";
import type boards from "../locales/en/boards.json";
import type common from "../locales/en/common.json";
import type components from "../locales/en/components.json";
import type config from "../locales/en/config.json";
import type errors from "../locales/en/errors.json";
import type global from "../locales/en/global.json";
import type keys from "../locales/en/keys.json";
import type knowledge from "../locales/en/knowledge.json";
import type projects from "../locales/en/projects.json";
import type providers from "../locales/en/providers.json";
import type sessions from "../locales/en/sessions.json";
import type settings from "../locales/en/settings.json";
import type shell from "../locales/en/shell.json";

declare module "i18next" {
  interface CustomTypeOptions {
    defaultNS: "common";
    resources: {
      common: typeof common;
      components: typeof components;
      shell: typeof shell;
      errors: typeof errors;
      accounts: typeof accounts;
      activities: typeof activities;
      projects: typeof projects;
      sessions: typeof sessions;
      knowledge: typeof knowledge;
      keys: typeof keys;
      providers: typeof providers;
      agents: typeof agents;
      global: typeof global;
      config: typeof config;
      boards: typeof boards;
      settings: typeof settings;
    };
  }
}
