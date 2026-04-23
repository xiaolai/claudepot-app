import { useEffect, useState } from "react";
import { api } from "../api";

/**
 * Reads the `activity_*` preference block. Refreshes on mount and
 * whenever `cp-activity-prefs-changed` fires (Settings dispatches
 * this after mutating the prefs). Consumers get the latest snapshot
 * without threading props through the tree.
 *
 * Failure-tolerant: if `preferencesGet` throws (non-Tauri env,
 * keychain locked at boot) the shape stays at its defaults, matching
 * the backend's `Preferences::Default`.
 */
export interface ActivityPrefs {
  enabled: boolean;
  hideThinking: boolean;
}

const DEFAULTS: ActivityPrefs = {
  enabled: false,
  // Matches claudepot::preferences::Preferences::default() — the
  // backend defaults this to true (privacy-forward).
  hideThinking: true,
};

export function useActivityPrefs(): ActivityPrefs {
  const [prefs, setPrefs] = useState<ActivityPrefs>(DEFAULTS);

  useEffect(() => {
    let cancelled = false;
    const load = () => {
      api
        .preferencesGet()
        .then((p) => {
          if (cancelled) return;
          setPrefs({
            enabled: p.activity_enabled,
            hideThinking: p.activity_hide_thinking,
          });
        })
        .catch(() => {
          // Swallow — default state already reflects the safe stance.
        });
    };

    load();
    const onChange = () => load();
    window.addEventListener("cp-activity-prefs-changed", onChange);
    return () => {
      cancelled = true;
      window.removeEventListener("cp-activity-prefs-changed", onChange);
    };
  }, []);

  return prefs;
}
