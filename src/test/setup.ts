// Vitest global setup — run before every test file.
//
// Adds jest-dom matchers (toBeInTheDocument, toBeDisabled, etc.) and makes
// sure the Tauri IPC is mocked by default so tests never accidentally hit
// a real invoke handler (which doesn't exist outside the webview anyway).

import "@testing-library/jest-dom/vitest";
import { afterEach, vi } from "vitest";
import { cleanup } from "@testing-library/react";
// Initialize the global i18next instance (module side effect) so any
// component using useTranslation renders real strings. jsdom reports
// navigator.language "en-US", so tests run in English by default and
// existing getByText assertions stay valid.
import "../lib/i18n";

// Node 26 exposes an experimental global `localStorage` that is undefined
// unless the process receives `--localstorage-file`. Use an explicit memory
// implementation so the test contract does not depend on Node's browser
// emulation or worker process flags.
class TestStorage implements Storage {
  private readonly values = new Map<string, string>();

  get length(): number {
    return this.values.size;
  }

  clear(): void {
    this.values.clear();
  }

  getItem(key: string): string | null {
    return this.values.get(String(key)) ?? null;
  }

  key(index: number): string | null {
    return Array.from(this.values.keys())[index] ?? null;
  }

  removeItem(key: string): void {
    this.values.delete(String(key));
  }

  setItem(key: string, value: string): void {
    this.values.set(String(key), String(value));
  }
}

const testLocalStorage = new TestStorage();
const testSessionStorage = new TestStorage();
vi.stubGlobal("localStorage", testLocalStorage);
vi.stubGlobal("sessionStorage", testSessionStorage);
Object.defineProperty(window, "localStorage", {
  configurable: true,
  value: testLocalStorage,
});
Object.defineProperty(window, "sessionStorage", {
  configurable: true,
  value: testSessionStorage,
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  testLocalStorage.clear();
  testSessionStorage.clear();
});

// Silence jsdom's "Not implemented: window.alert" for confirm() prompts.
globalThis.confirm = () => true;

// jsdom doesn't implement Element.scrollIntoView — stub it so components
// that call it (e.g. CommandPalette keyboard navigation) don't crash.
if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = () => {};
}

// Tauri's event bus, mocked once for every test file.
//
// `listen()` reaches `window.__TAURI_INTERNALS__`, which does not exist
// outside the webview, so a component that subscribes on mount throws
// inside a passive effect. React swallows that into an unhandled
// rejection — which vitest reports at the END of the run, attributed to
// whichever file happened to be running, while every test in it still
// reports green. That is the worst possible shape for a failure: a red
// exit code with 1347 passes above it and a stack pointing at a file
// that did nothing wrong.
//
// Five test files had each hand-rolled this mock; the sixth component to
// call `listen` (`RemoteWindowChip`, reached through `AppStatusBar`)
// found the one test file that had not. A mock every renderer needs
// belongs here, not copied into each caller. Files that need to assert
// on their own `listen` still declare a local `vi.mock`, which wins.
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
  emit: vi.fn().mockResolvedValue(undefined),
  once: vi.fn().mockResolvedValue(() => {}),
}));

// The OS opener, for the same reason and with the same shape.
//
// This is the second Tauri API a component called that no test file had
// mocked, so it is a class rather than an incident: anything reached
// through `@tauri-apps/*` throws outside the webview, and a component
// that calls one in an effect or a handler turns that into an unhandled
// rejection — reported at the END of the run, attributed to whichever
// file was executing, with every test above still green.
//
// The resolved promise is load-bearing: `ExternalLink` does
// `openUrl(href).catch(…)`, so a bare `vi.fn()` returning `undefined`
// throws on `.catch` at click time. A mock has to keep the contract the
// caller relies on, not merely exist.
vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: vi.fn().mockResolvedValue(undefined),
  openPath: vi.fn().mockResolvedValue(undefined),
  revealItemInDir: vi.fn().mockResolvedValue(undefined),
}));
