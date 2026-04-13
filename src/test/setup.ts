// Vitest global setup — run before every test file.
//
// Adds jest-dom matchers (toBeInTheDocument, toBeDisabled, etc.) and makes
// sure the Tauri IPC is mocked by default so tests never accidentally hit
// a real invoke handler (which doesn't exist outside the webview anyway).

import "@testing-library/jest-dom/vitest";
import { afterEach, vi } from "vitest";
import { cleanup } from "@testing-library/react";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

// Silence jsdom's "Not implemented: window.alert" for confirm() prompts.
globalThis.confirm = () => true;
