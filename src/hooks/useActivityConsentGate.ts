import { useEffect, useState } from "react";
import { api } from "../api";

/**
 * Live Activity consent gate, extracted from AppShell. On cold launch
 * we ask the backend once whether the consent modal still needs to
 * fire. `open === true` = modal open; `false` = either already
 * accepted/declined, or the prefs fetch failed (fail-closed: no
 * modal means no surprise reads). When consent was already granted
 * and activity is enabled, this is also the path that starts the
 * live runtime.
 *
 * The former sidebar "Off" chip (for the dedicated Activity row)
 * went away with the C-1 A consolidation — Sessions' Live filter
 * covers the same "is the runtime on" signal indirectly.
 */
export function useActivityConsentGate(): {
  open: boolean;
  dismiss: () => void;
} {
  const [open, setOpen] = useState(false);

  useEffect(() => {
    let cancelled = false;
    api
      .preferencesGet()
      .then((p) => {
        if (cancelled) return;
        if (!p.activity_consent_seen) setOpen(true);
        else if (p.activity_enabled) {
          api.sessionLiveStart().catch(() => {});
        }
      })
      .catch(() => {
        // Prefs fetch failed (non-Tauri env). Leave modal closed.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return { open, dismiss: () => setOpen(false) };
}
