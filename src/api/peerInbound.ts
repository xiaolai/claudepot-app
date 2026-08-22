// The time-boxed remote-control window — frontend bindings for the
// `peer_inbound_*` Tauri commands. See
// `src-tauri/src/commands/peer_inbound.rs` and the "## Peer messaging"
// section of AGENTS.md.

import { invoke } from "@tauri-apps/api/core";

/** What `crossSessionInbound` currently holds. `absent` and `invalid`
 *  are separate on purpose: absent means CC's own default is in play,
 *  invalid means the file holds something CC rejects — different
 *  problems, different fixes. */
export type PeerInboundObserved =
  | "accept"
  | "hold"
  | "refuse"
  | "absent"
  | "invalid";

export type PeerInboundState = {
  /** Peer messages deliver without asking, however that came about. */
  open: boolean;
  /** Open AND nothing holds a deadline on it — no timer will close it.
   *  Rendered differently from a managed window for exactly that
   *  reason. */
  unmanagedOpen: boolean;
  remainingSecs: number | null;
  expiresAt: string | null;
  observed: PeerInboundObserved;
};

export const peerInboundApi = {
  /** Read-only; does not reconcile. The orchestrator owns expiry. */
  peerInboundState: (): Promise<PeerInboundState> =>
    invoke("peer_inbound_state"),

  peerInboundGrant: (
    durationSecs: number,
    reason?: string,
  ): Promise<PeerInboundState> =>
    invoke("peer_inbound_grant", { durationSecs, reason }),

  peerInboundRevoke: (): Promise<PeerInboundState> =>
    invoke("peer_inbound_revoke"),
};
