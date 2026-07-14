// Vitest global setup — run before every test file.
//
// Adds jest-dom matchers (toBeInTheDocument, toBeDisabled, etc.) and makes
// sure the Tauri IPC is mocked by default so tests never accidentally hit
// a real invoke handler (which doesn't exist outside the webview anyway).

import "@testing-library/jest-dom/vitest";
import { afterEach, vi } from "vitest";
import { cleanup } from "@testing-library/react";

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
