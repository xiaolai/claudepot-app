import { useRef } from "react";
import type { Event as TauriEvent } from "@tauri-apps/api/event";

import type { RelayLive } from "../api";
import { i18n } from "../lib/i18n";
import { useEmit } from "../providers/AppStateProvider";
import { useTauriEvents } from "./useTauriEvent";

/** Per-upstream codes the banner owns (`useRelayIssues`). */
const BLOCKING: ReadonlySet<string> = new Set(["owner_conflict", "identity_changed"]);

/**
 * The relay's **durable** tier for upstream errors that do not need the
 * user. What arrives is each upstream's **credential health** — the
 * `credential_error` its status reports (`RelayStatusEvent::from_live`):
 * a token acquisition or renewal that failed (`credential_unavailable`,
 * `upstream_unreachable`, `upstream_timeout`, `upstream_redirect`) and
 * will be retried in the background, or a token the upstream rejected
 * twice (`upstream_credential_rejected`). Each new error on each
 * upstream lands once in the notification log (`relayError`, P3 ambient
 * — log-only, no toast or banner).
 *
 * Per-request failures are **not** in that stream: a 429 only sets the
 * upstream's cooldown, and a transport error on one request goes back
 * to that client. Neither reaches this hook, so it promises neither.
 *
 * Tracked per upstream, not from the aggregate `lastError`: that names
 * one upstream's error, so while upstream A stayed faulty a failure on
 * B produced no row at all.
 *
 * Blocking conditions (`owner_conflict`, `identity_changed`) are the
 * banner's (`useRelayIssues`) and are not logged twice here.
 *
 * Subscribes to `relay-status` only. Upstream errors are live-relay
 * fields, so a poll would add nothing.
 */
export function useRelayErrorNotifications(): void {
  const emit = useEmit();
  // The last code seen per upstream id, so a ticker repeating the same
  // error is one row, and the same error coming back after a clean tick
  // is a new one.
  const last = useRef(new Map<string, string>());

  useTauriEvents({
    "relay-status": (ev: TauriEvent<RelayLive>) => {
      const live = ev.payload;
      if (!live) return;
      const next = new Map<string, string>();
      if (live.serving) {
        for (const u of live.upstreams) if (u.error) next.set(u.id, u.error);
      }
      const prev = last.current;
      last.current = next;
      for (const [id, code] of next) {
        if (prev.get(id) === code || BLOCKING.has(code)) continue;
        void emit({
          category: "relayError",
          title: i18n.t("relayLog.title"),
          body: i18n.t("relayLog.body", { code }),
          dedupeKey: `relay:${id}:${code}`,
        });
      }
    },
  });
}
