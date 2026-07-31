/**
 * React binding for {@link enabledSections}.
 *
 * Every component that renders a list of sections uses this, so the
 * sidebar, the palette, the shortcuts modal, and the launch picker
 * cannot disagree about which sections exist. See
 * `lib/optionalSections.ts` for why that matters.
 */

import { useSyncExternalStore } from "react";
import {
  enabledSections,
  OPTIONAL_SECTIONS_EVENT,
} from "../lib/optionalSections";
import type { SectionDef } from "../sections/registry";

function subscribe(onChange: () => void): () => void {
  window.addEventListener(OPTIONAL_SECTIONS_EVENT, onChange);
  // `storage` fires when ANOTHER window changes localStorage, which is
  // how a second Claudepot window stays in sync. The custom event
  // covers this window, where `storage` deliberately does not fire.
  window.addEventListener("storage", onChange);
  return () => {
    window.removeEventListener(OPTIONAL_SECTIONS_EVENT, onChange);
    window.removeEventListener("storage", onChange);
  };
}

// `useSyncExternalStore` compares snapshots by identity, so a fresh
// array every call would loop forever. Recompute only when something
// actually fires, and hand back the same reference in between.
let cached: readonly SectionDef[] = enabledSections();
let dirty = false;

function markDirty() {
  dirty = true;
}
if (typeof window !== "undefined") {
  window.addEventListener(OPTIONAL_SECTIONS_EVENT, markDirty);
  window.addEventListener("storage", markDirty);
}

function getSnapshot(): readonly SectionDef[] {
  if (dirty) {
    cached = enabledSections();
    dirty = false;
  }
  return cached;
}

export function useEnabledSections(): readonly SectionDef[] {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
