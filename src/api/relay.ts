// The relay server — frontend bindings for the `relay_*` Tauri
// commands. See `src-tauri/src/commands/relay.rs`, `relay_server.rs`,
// and `claudepot-core::relay::service`, which `claudepot relay …`
// shares.
//
// No credential crosses this bridge. The pairing code does: it is
// shown once for the user to type on the other machine, is single-use,
// and expires in minutes.

import { invoke } from "@tauri-apps/api/core";

import type { RemoteExposure } from "./remote";

/** The one condition per upstream that needs the user, from
 *  `relay-status`. Every other upstream error goes to the log. */
export type RelayBlocking = "owner_conflict" | "identity_changed";

export type RelayUpstreamLive = {
  id: string;
  ready: boolean;
  /** A stable refusal code (`rate_limited`, `owner_conflict`, …). */
  error: string | null;
};

/** The `relay-status` event payload — `relay_server::RelayStatusEvent`. */
export type RelayLive = {
  serving: boolean;
  activeStreams: number;
  blocking: RelayBlocking | null;
  lastError: string | null;
  upstreams: RelayUpstreamLive[];
};

export type RelayDayUsage = {
  requests: number;
  failed: number;
  /** Always an estimate (`≈`); meaningless when `unpriced === requests`. */
  costUsd: number;
  unpriced: number;
  familyEstimate: boolean;
};

export type RelayDevice = {
  id: string;
  name: string;
  createdAt: string;
  revokedAt: string | null;
  includeFuture: boolean;
  /** Upstream ids this device has accepted. */
  grants: string[];
  lastSeen: string | null;
  today: RelayDayUsage;
};

/** `account` from `relay publish`; `route` from `relay publish-route`
 *  (a provider route served by pass-through). */
export type RelayPublicationKind = "account" | "route";

export type RelayPublication = {
  id: string;
  generation: number;
  kind: RelayPublicationKind;
  /** `kind === "account"` only; `null` when the account is no longer
   *  registered here. */
  accountEmail: string | null;
  /** `kind === "route"` only. */
  routeId: string | null;
  /** `kind === "route"` only; `null` when the route was deleted. */
  routeName: string | null;
};

/** Why an account cannot be published. */
export type RelayIneligible = "cc_active" | "no_credentials";

export type RelayAccount = {
  email: string;
  ineligible: RelayIneligible | null;
};

export type RelayWarning = { upstream: string; code: string };

export type RelayStatus = {
  /** A relay serves on this machine — this process or a terminal. */
  serving: boolean;
  /** This process serves. `serving && !runningHere` means a CLI relay
   *  owns it: Stop here cannot touch it, and changes are refused. */
  runningHere: boolean;
  addr: string | null;
  bind: string;
  port: number;
  exposure: RemoteExposure | null;
  bindError: string | null;
  caFingerprint: string | null;
  configRecovered: boolean;
  devicesRecovered: boolean;
  /** The usage ledger could not be read — per-device "today" is unknown, not zero. */
  usageUnavailable: boolean;
  publications: RelayPublication[];
  devices: RelayDevice[];
  accounts: RelayAccount[];
  lastError: string | null;
  /** `relay_ca_key_unavailable`, … — set when TLS material refused a start. */
  lastErrorCode: string | null;
  warnings: RelayWarning[];
  live: RelayLive;
  /** This process serves but a terminal `claudepot relay revoke` cannot
   *  reach it, and why (`relay::revoke_socket::Unavailable::code`).
   *  Revoking from this window still works. */
  liveRevokeUnavailable: RelayLiveRevokeUnavailable | null;
  /** A terminal relay serves and has left a revoke socket, so Revoke
   *  here can go through it. A hint, not a promise: the revoke itself
   *  reports a socket that does not answer. */
  revokeSocketPresent: boolean;
};

export type RelayLiveRevokeUnavailable =
  | "unsupported"
  | "path_too_long"
  | "path_occupied"
  | "unsafe_directory"
  | "bind_failed";

/** `alreadyRevoked`: it was revoked before this call; nothing changed. */
export type RelayRevokeReceipt = { alreadyRevoked: boolean };

export type RelayPairOffer = {
  code: string;
  expiresInSecs: number;
  caFingerprint: string;
};

/** Why an approved upstream cannot serve a run now —
 *  `relay::select::Ineligible::code`. */
export type RelayClientIneligible =
  | "unreachable"
  | "server_id_changed"
  | "not_approved"
  | "other_target"
  | "not_offered"
  | "fingerprint_changed"
  | "not_accepted"
  | "credential_not_ready"
  | "owner_conflict"
  | "no_capacity"
  | "cooldown"
  | "saturated";

/** Why a paired server's probe failed —
 *  `relay::client_ops::ProbeFailure::code`. Never the relay's text. */
export type RelayProbeError =
  | "fingerprint_mismatch"
  | "bad_ca"
  | "transport"
  | "refused"
  | "bad_response"
  | "tls"
  | "not_paired"
  | "unpinned"
  | "bad_address"
  | "no_token"
  | "credential_store_locked"
  | "credential_store";

/** One upstream of a paired relay server, as this client sees it now. */
export type RelayClientUpstream = {
  id: string;
  eligible: boolean;
  /** Set when eligible. */
  tier: "measured" | "connect" | null;
  /** `tier === "measured"` only. */
  tierModel: string | null;
  /** `tier === "measured"` only. */
  ttfbMs: number | null;
  /** Compared only within a tier. */
  scoreMs: number | null;
  /** Set when not eligible. */
  reason: RelayClientIneligible | null;
  /** `reason === "cooldown"` only. */
  cooldownSecs: number | null;
  /** `reason === "saturated"` only. */
  saturatedWindow: "5-hour" | "7-day" | null;
  /** `reason === "saturated"` only, in percent. */
  saturatedUtilization: number | null;
};

/** A relay server this machine paired with (`relay-servers.json`),
 *  probed now. Read-only — pairing and launching stay in the CLI. */
export type RelayClientServer = {
  /** Internal key; never shown in primary UI. */
  serverId: string;
  name: string;
  baseUrl: string;
  reachable: boolean;
  error: RelayProbeError | null;
  rttMs: number | null;
  defaultUpstream: string | null;
  upstreams: RelayClientUpstream[];
};

export const relayApi = {
  relayStatus: (): Promise<RelayStatus> => invoke("relay_status"),

  /** Resolves with the listening address. Idempotent while serving here. */
  relayStart: (): Promise<string> => invoke("relay_start"),

  /** `false` when nothing was serving in this process. */
  relayStop: (): Promise<boolean> => invoke("relay_stop"),

  /** Refused while serving. Resolves with the exposure word. */
  relaySetBind: (address: string, port?: number): Promise<RemoteExposure> =>
    invoke("relay_set_bind", { address, port: port ?? null }),

  /** Resolves with the upstream id. Refuses Claude Code's active login. */
  relayPublish: (email: string, id?: string): Promise<string> =>
    invoke("relay_publish", { email, id: id ?? null }),

  relayUnpublish: (id: string): Promise<void> => invoke("relay_unpublish", { id }),

  /** Through the relay serving here, the revoke socket of one serving
   *  from a terminal, or offline. Rejects `relay.revoke_outcome_unknown`
   *  when a terminal relay took the request and did not confirm it. */
  relayRevokeDevice: (id: string): Promise<RelayRevokeReceipt> =>
    invoke("relay_revoke_device", { id }),

  relayPairCode: (includeFuture: boolean): Promise<RelayPairOffer> =>
    invoke("relay_pair_code", { includeFuture }),

  /** Paired relay servers, each probed now. Can take seconds: every
   *  server is probed concurrently, each request bounded by the relay
   *  client's timeout. */
  relayClientServers: (): Promise<RelayClientServer[]> =>
    invoke("relay_client_servers"),
};
