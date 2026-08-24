// The LAN appliance — frontend bindings for the `remote_*` Tauri
// commands. See `src-tauri/src/commands/remote.rs`, `remote_server.rs`,
// and the "## Remote control" section of AGENTS.md.

import { invoke } from "@tauri-apps/api/core";

/** What a bind address exposes. Stable wire words from
 *  `remote::service::exposure_word`, not an enum's `Debug`. */
export type RemoteExposure = "loopback" | "private_network" | "every_interface";

export type RemoteDevice = {
  id: string;
  name: string;
  createdAt: string;
  lastSeen: string | null;
  revokedAt: string | null;
  /** `null` for a paired device (valid until revoked); a timestamp for
   *  a password-issued session. */
  expiresAt: string | null;
};

export type RemoteStatus = {
  /** The stored preference. **Not liveness** — see `serving`. */
  enabled: boolean;
  /**
   * A server is up somewhere on this machine, read from the heartbeat.
   *
   * `enabled` survives a `kill -9`, which is why these are two fields.
   * True of a `claudepot remote serve` running in a terminal too.
   */
  serving: boolean;
  /** This process is the one serving. `serving && !runningHere` means a
   *  CLI server owns the port and Stop here cannot touch it. */
  runningHere: boolean;
  url: string | null;
  bind: string;
  port: number;
  /** `null` when the address is refused; `bindError` says why. */
  exposure: RemoteExposure | null;
  bindError: string | null;
  requiresTls: boolean;
  passwordSet: boolean;
  totpEnabled: boolean;
  passkeys: number;
  /**
   * May a paired device answer a permission prompt?
   *
   * The one capability on this surface that GRANTS rather than reads.
   * With it off the panel still lists sessions, reads transcripts and
   * sends prompts; a permission prompt is drawn at the machine.
   */
  approvalsEnabled: boolean;
  /** The config file was unreadable and was reset — the login throttle
   *  and the spent-TOTP high-water mark were in it. */
  configRecovered: boolean;
  /** The device file was unreadable and was reset — every previous
   *  revocation is gone, and revoking is refused until re-paired. */
  devicesRecovered: boolean;
  /** Why the last start failed, or why a running server died. */
  lastError: string | null;
  /** Non-empty means approval-from-the-phone is off while the rest
   *  works — the user would otherwise find out by tapping Allow and
   *  having nothing happen. */
  warnings: string[];
  devices: RemoteDevice[];
  activeDevices: number;
};

export const remoteApi = {
  remoteStatus: (): Promise<RemoteStatus> => invoke("remote_status"),

  /** Turns the preference on. Does not start the server. */
  remoteEnable: (bind?: string, port?: number): Promise<void> =>
    invoke("remote_enable", { bind: bind ?? null, port: port ?? null }),

  /** Turns the preference off AND stops a server this process runs. */
  remoteDisable: (): Promise<void> => invoke("remote_disable"),

  /** The password crosses INTO Rust and is zeroized there. Nothing
   *  returns it — see `rules/architecture.md` on secret direction. */
  remoteSetPassword: (password: string): Promise<void> =>
    invoke("remote_set_password", { password }),

  /**
   * Allow or refuse answering permission prompts from a paired device.
   *
   * Takes effect immediately, including on a server already running —
   * the hook reads the preference on every invocation.
   */
  remoteSetApprovals: (enabled: boolean): Promise<void> =>
    invoke("remote_set_approvals", { enabled }),

  /** Resolves with the URL. Starting while already running returns the
   *  running server's URL rather than failing on a busy port. */
  remoteStart: (): Promise<string> => invoke("remote_start"),

  /** `false` when nothing was running in this process. */
  remoteStop: (): Promise<boolean> => invoke("remote_stop"),

  /** Returns how many devices changed. */
  remoteRevokeAll: (): Promise<number> => invoke("remote_revoke_all"),

  /** `false` when that device was already revoked. */
  remoteRevokeDevice: (id: string): Promise<boolean> =>
    invoke("remote_revoke_device", { id }),
};
