import { useCallback, useEffect, useRef, useState } from "react";
import type { Event as TauriEvent } from "@tauri-apps/api/event";

import { relayApi, type RelayLive, type RelayStatus } from "../api";
import { useTauriEvents } from "./useTauriEvent";

/** The machine-wide read. Slow on purpose: `relay-status` carries every
 *  change of a relay this process serves, so the poll only has to catch
 *  a relay started or stopped from a terminal. */
const POLL_MS = 10_000;

/** The ambient state of this machine's relay. */
export type RelayAmbient = {
  /** Serving here or from a terminal. */
  serving: boolean;
  live: RelayLive;
  /** `relay_ca_key_unavailable` etc., from the last start here. */
  lastErrorCode: string | null;
  /** The newest read failed, so this is the last state that was read
   *  (or an event applied since). Cleared by the next success or event. */
  stale: boolean;
};

function fromStatus(s: RelayStatus): RelayAmbient {
  return { serving: s.serving, live: s.live, lastErrorCode: s.lastErrorCode, stale: false };
}

/**
 * The relay's ambient state for the status-bar chip and the blocking
 * banner: `relay_status` on mount and every 10 s, with each
 * `relay-status` event applied as it lands.
 *
 * `null` only until the first read lands. A failed read after that
 * keeps the last known state and marks it `stale`: `relay_status` can
 * fail on the account store or a relay file while the listener keeps
 * serving, so a failure is not proof of shutdown, and dropping the chip
 * and the blocking banners on one would hide a relay that still serves.
 */
export function useRelayStatus(): RelayAmbient | null {
  const [state, setState] = useState<RelayAmbient | null>(null);
  // An older reply landing after a newer one must not win.
  const seq = useRef(0);
  // Bumped by every `relay-status` event. A read that began before one
  // is older than it for the live fields only: the event carries no
  // `lastErrorCode`, so discarding the whole read would drop the CA
  // fault the banner needs — and under streaming, every read.
  const liveSeq = useRef(0);
  const alive = useRef(true);
  // A poll never starts while a read is pending. Each new read
  // invalidated the one before it (`seq`), so a read slower than the
  // poll interval was always overtaken and none ever landed.
  const pending = useRef(false);

  const refresh = useCallback(async () => {
    if (pending.current) return;
    pending.current = true;
    const mine = ++seq.current;
    const liveAtStart = liveSeq.current;
    try {
      const s = await relayApi.relayStatus();
      if (!alive.current || mine !== seq.current) return;
      const overtaken = liveAtStart !== liveSeq.current;
      setState((prev) =>
        overtaken && prev ? { ...prev, lastErrorCode: s.lastErrorCode } : fromStatus(s),
      );
    } catch {
      // A failed read after an event says nothing about the event's state.
      if (alive.current && mine === seq.current && liveAtStart === liveSeq.current) {
        setState((prev) => (prev ? { ...prev, stale: true } : prev));
      }
    } finally {
      pending.current = false;
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    void refresh();
    const id = window.setInterval(() => void refresh(), POLL_MS);
    return () => {
      alive.current = false;
      window.clearInterval(id);
    };
  }, [refresh]);

  useTauriEvents({
    "relay-status": (ev: TauriEvent<RelayLive>) => {
      if (!ev.payload) return;
      const live = ev.payload;
      // An event outranks any read still in flight — for the live
      // fields; the read still brings `lastErrorCode` (see `liveSeq`).
      liveSeq.current++;
      setState((prev) => ({
        // A stop from this process is not proof nothing serves: a
        // terminal relay cannot start while ours runs, so after our stop
        // the next poll is the authority.
        serving: live.serving,
        live,
        lastErrorCode: prev?.lastErrorCode ?? null,
        stale: false,
      }));
    },
  });

  return state;
}
