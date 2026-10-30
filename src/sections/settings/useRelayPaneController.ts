import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { relayApi, type RelayDevice, type RelayPairOffer, type RelayStatus } from "../../api";
import { renderError } from "../../lib/i18n-error";
import { useGlobalShortcuts } from "../../hooks/useGlobalShortcuts";

/** Poll while open: a terminal relay can start or stop under us, and
 *  devices' last-seen and today's usage move while it serves. */
const POLL_MS = 4000;
/** How often an open pairing code re-reads the clock. Its expiry is
 *  shown in whole minutes, so a second is precise enough to retire it
 *  on time and cheap enough to run only while a code is shown. */
const OFFER_TICK_MS = 1000;

/** A pairing code, with the wall-clock moment it stops working. */
export type RelayOfferState = RelayPairOffer & { deadline: number };

export type RelayPaneController = {
  status: RelayStatus | null;
  /** The last read's failure. Kept beside `status`, never instead of
   *  it: a failed poll is not proof the relay changed. */
  loadError: string | null;
  busy: string | null;
  bind: string;
  setBind: (v: string) => void;
  port: string;
  setPort: (v: string) => void;
  includeFuture: boolean;
  setIncludeFuture: (v: boolean) => void;
  offer: RelayOfferState | null;
  /** Milliseconds left on `offer`; `0` once it has expired. */
  offerRemainingMs: number;
  confirmRevoke: RelayDevice | null;
  setConfirmRevoke: (d: RelayDevice | null) => void;
  /** `okMsg` may read the verb's result, for a toast that depends on it.
   *  `errScope` prefixes a failure toast; the default says the relay was
   *  not changed, which is wrong for a revoke whose outcome is unknown. */
  run: <T>(
    key: string,
    fn: () => Promise<T>,
    okMsg?: string | ((r: T) => string),
    errScope?: string,
  ) => Promise<void>;
  makePairCode: () => Promise<void>;
};

/**
 * Settings → Relay's state and verbs, apart from its markup.
 *
 * Four ordering rules, each of which a naive version got wrong:
 *
 * - **A reply is applied only if it answers the newest read.** ⌘R and
 *   post-action refreshes overlap polls; an older reply landing last
 *   would restore state the user just changed. The refresh after a
 *   mutation is itself the newest read, so it is what discards every
 *   read still in flight from before the change.
 * - **The interval never overlaps a read in flight.** It skips its tick
 *   instead. Every tick used to start a read that invalidated the last,
 *   so once a read took longer than the interval every reply was
 *   discarded and the pane never updated again.
 * - **The address fields are seeded once, by a flag, not by emptiness.**
 *   Treating `""` as "not yet loaded" made the next poll refill a field
 *   the user had just cleared to retype.
 * - **A pairing code carries its deadline**, counted from when the
 *   request was sent. The server starts the clock when it mints the
 *   code, which is before the reply arrives; counting from the reply
 *   would show a dead code as live for as long as delivery took.
 */
export function useRelayPaneController(
  pushToast?: (kind: "info" | "error", text: string) => void,
): RelayPaneController {
  const { t } = useTranslation("settings");
  const [status, setStatus] = useState<RelayStatus | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [bind, setBind] = useState("");
  const [port, setPort] = useState("");
  const [includeFuture, setIncludeFuture] = useState(false);
  const [offer, setOffer] = useState<RelayOfferState | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const [confirmRevoke, setConfirmRevoke] = useState<RelayDevice | null>(null);

  const seq = useRef(0);
  /** Reads in flight, so the interval can wait its turn. */
  const pending = useRef(0);
  const seeded = useRef(false);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const load = useCallback(async () => {
    const mine = ++seq.current;
    pending.current++;
    try {
      const s = await relayApi.relayStatus();
      if (!alive.current || mine !== seq.current) return;
      setStatus(s);
      setLoadError(null);
      if (!seeded.current) {
        seeded.current = true;
        setBind(s.bind);
        setPort(String(s.port));
      }
      // A code minted by a relay that has since stopped is dead.
      if (!s.runningHere) setOffer(null);
    } catch (e) {
      if (!alive.current || mine !== seq.current) return;
      setLoadError(renderError(e, t("relay.loadFailed")));
    } finally {
      pending.current--;
    }
  }, [t]);
  // ⌘R — this surface's refresh; see `refresh` in sections/registry.tsx.
  useGlobalShortcuts({ onRefresh: () => void load() });

  useEffect(() => {
    void load();
    const id = window.setInterval(() => {
      if (pending.current === 0) void load();
    }, POLL_MS);
    return () => window.clearInterval(id);
  }, [load]);

  // Tick only while a live code is on screen, and stop at its deadline.
  const offerDeadline = offer?.deadline ?? null;
  useEffect(() => {
    if (offerDeadline === null) return;
    setNow(Date.now());
    const id = window.setInterval(() => {
      const n = Date.now();
      setNow(n);
      if (n >= offerDeadline) window.clearInterval(id);
    }, OFFER_TICK_MS);
    return () => window.clearInterval(id);
  }, [offerDeadline]);

  const run = useCallback(
    async <T>(key: string, fn: () => Promise<T>, okMsg?: string | ((r: T) => string), errScope?: string) => {
      setBusy(key);
      try {
        const r = await fn();
        const msg = typeof okMsg === "function" ? okMsg(r) : okMsg;
        if (msg) pushToast?.("info", msg);
        await load();
      } catch (e) {
        pushToast?.("error", renderError(e, errScope ?? t("relay.actionFailed")));
        // A failure is not proof nothing changed — a revoke a terminal
        // relay never confirmed may have landed — so read again now
        // rather than at the next poll.
        await load();
      } finally {
        if (alive.current) setBusy(null);
      }
    },
    [load, pushToast, t],
  );

  const makePairCode = useCallback(
    () =>
      run("pair", async () => {
        // Conservative: the server's clock started at or after this.
        const sent = Date.now();
        const o = await relayApi.relayPairCode(includeFuture);
        setOffer({ ...o, deadline: sent + o.expiresInSecs * 1000 });
      }),
    [run, includeFuture],
  );

  return {
    status,
    loadError,
    busy,
    bind,
    setBind,
    port,
    setPort,
    includeFuture,
    setIncludeFuture,
    offer,
    offerRemainingMs: offer ? Math.max(0, offer.deadline - now) : 0,
    confirmRevoke,
    setConfirmRevoke,
    run,
    makePairCode,
  };
}
