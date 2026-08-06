/**
 * i18n core — the ONE place the i18next instance is created and
 * initialized. Initialization is synchronous (static bundled
 * catalogs, `initImmediate: false`) and happens at module load, so
 * any component or plain function can translate from its very first
 * render with no async gate and no flash of translation keys.
 *
 * Locale resolution has two tiers:
 *
 *   1. `preferences.json` (`Preferences.locale`) is authoritative —
 *      it is what the Rust side (tray, app menu, OS banners) reads.
 *   2. `localStorage[LOCALE_KEY]` is a boot mirror of that value so
 *      the first paint renders in the right language without waiting
 *      for the `preferences_get` IPC round-trip. Same pattern as the
 *      pre-mount `cp-theme` read in index.html.
 *
 * `null` preference = follow the OS language. The mirror stores the
 * *preference* (which may be null/absent), never the resolved locale
 * — resolving at read time is what keeps "follow system" following.
 *
 * The supported-locale list is duplicated in
 * `src-tauri/src/commands/preferences.rs` (`preferences_set_locale`'s
 * allowlist). Two locales don't warrant a codegen bridge; if the list
 * grows past a handful, generate both from one source.
 */
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { LOCALE_KEY } from "./storageKeys";
import enAccounts from "../locales/en/accounts.json";
import enActivities from "../locales/en/activities.json";
import enAgents from "../locales/en/agents.json";
import enBoards from "../locales/en/boards.json";
import enCommon from "../locales/en/common.json";
import enComponents from "../locales/en/components.json";
import enConfig from "../locales/en/config.json";
import enErrors from "../locales/en/errors.json";
import enGlobal from "../locales/en/global.json";
import enKeys from "../locales/en/keys.json";
import enKnowledge from "../locales/en/knowledge.json";
import enProjects from "../locales/en/projects.json";
import enProviders from "../locales/en/providers.json";
import enSessions from "../locales/en/sessions.json";
import enSettings from "../locales/en/settings.json";
import enShell from "../locales/en/shell.json";
import zhAccounts from "../locales/zh-CN/accounts.json";
import zhActivities from "../locales/zh-CN/activities.json";
import zhAgents from "../locales/zh-CN/agents.json";
import zhBoards from "../locales/zh-CN/boards.json";
import zhCommon from "../locales/zh-CN/common.json";
import zhComponents from "../locales/zh-CN/components.json";
import zhConfig from "../locales/zh-CN/config.json";
import zhErrors from "../locales/zh-CN/errors.json";
import zhGlobal from "../locales/zh-CN/global.json";
import zhKeys from "../locales/zh-CN/keys.json";
import zhKnowledge from "../locales/zh-CN/knowledge.json";
import zhProjects from "../locales/zh-CN/projects.json";
import zhProviders from "../locales/zh-CN/providers.json";
import zhSessions from "../locales/zh-CN/sessions.json";
import zhSettings from "../locales/zh-CN/settings.json";
import zhShell from "../locales/zh-CN/shell.json";

export const SUPPORTED_LOCALES = ["en", "zh-CN"] as const;
export type SupportedLocale = (typeof SUPPORTED_LOCALES)[number];

/** Valid `labelKey` values for a section-registry entry — keeps the
 *  registry compile-locked to the shell catalog: adding a section
 *  without a catalog entry is a type error, not a runtime English
 *  leak. Unprefixed (no `shell:`) — resolve with a shell-bound `t`
 *  (`useTranslation("shell")`) or `i18n.t(key, { ns: "shell" })`. */
export type SectionLabelKey =
  `sections.${keyof (typeof enShell)["sections"] & string}`;

/** Resolve a persisted preference (null = follow system) to a
 *  concrete supported locale. */
export function resolveLocale(
  pref: string | null | undefined,
): SupportedLocale {
  if (pref && (SUPPORTED_LOCALES as readonly string[]).includes(pref)) {
    return pref as SupportedLocale;
  }
  // Follow system. jsdom reports "en-US" under Vitest, so tests run
  // in English unless they opt into zh-CN explicitly.
  const nav = typeof navigator !== "undefined" ? navigator.language : "en";
  return /^zh/i.test(nav) ? "zh-CN" : "en";
}

function storedLocalePref(): string | null {
  try {
    return localStorage.getItem(LOCALE_KEY);
  } catch {
    return null;
  }
}

i18n.use(initReactI18next).init({
  lng: resolveLocale(storedLocalePref()),
  fallbackLng: "en",
  defaultNS: "common",
  ns: [
    "common",
    "components",
    "shell",
    "errors",
    "accounts",
    "activities",
    "projects",
    "sessions",
    "knowledge",
    "keys",
    "providers",
    "agents",
    "global",
    "config",
    "boards",
    "settings",
  ],
  resources: {
    en: {
      common: enCommon,
      components: enComponents,
      shell: enShell,
      errors: enErrors,
      accounts: enAccounts,
      activities: enActivities,
      projects: enProjects,
      sessions: enSessions,
      knowledge: enKnowledge,
      keys: enKeys,
      providers: enProviders,
      agents: enAgents,
      global: enGlobal,
      config: enConfig,
      boards: enBoards,
      settings: enSettings,
    },
    "zh-CN": {
      common: zhCommon,
      components: zhComponents,
      shell: zhShell,
      errors: zhErrors,
      accounts: zhAccounts,
      activities: zhActivities,
      projects: zhProjects,
      sessions: zhSessions,
      knowledge: zhKnowledge,
      keys: zhKeys,
      providers: zhProviders,
      agents: zhAgents,
      global: zhGlobal,
      config: zhConfig,
      boards: zhBoards,
      settings: zhSettings,
    },
  },
  // React escapes interpolated values itself; double-escaping renders
  // literal entities.
  interpolation: { escapeValue: false },
  // All catalogs are bundled statically — init must complete within
  // this synchronous module evaluation, before React's first render.
  // (i18next ≤v25 called this `initImmediate`.)
  initAsync: false,
});

/**
 * Apply a locale preference (null = follow system) to the running
 * instance and refresh the boot mirror. Called at boot with the
 * authoritative `Preferences.locale`, and from the Settings → General
 * picker after a successful `preferences_set_locale`.
 */
export async function applyLocalePreference(
  pref: string | null,
): Promise<void> {
  try {
    if (pref == null) localStorage.removeItem(LOCALE_KEY);
    else localStorage.setItem(LOCALE_KEY, pref);
  } catch {
    // Mirror is best-effort; preferences.json stays authoritative and
    // the next boot's preferences_get reconcile covers the gap.
  }
  const next = resolveLocale(pref);
  if (i18n.resolvedLanguage !== next) {
    await i18n.changeLanguage(next);
  }
}

/** BCP-47 tag of the active UI language — the one argument every
 *  `Intl`/`toLocaleString` call site should pass. */
export function getActiveLocale(): SupportedLocale {
  return resolveLocale(i18n.resolvedLanguage);
}

export { i18n };
export default i18n;
