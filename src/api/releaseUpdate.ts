// Channel-aware self-updater bindings for Claudepot's own app bundle.
//
// These wrap the Rust `release_*` commands. They are distinct from
// `updatesApi` (api/updates.ts), which manages Claude Code's CLI and
// Desktop — different feature entirely.
//
// Why these are Rust commands and not the JS `@tauri-apps/plugin-
// updater`: the JS plugin's `check()` cannot override the manifest
// endpoint (`CheckOptions` has no `endpoints` field). Only the Rust
// `UpdaterBuilder` can, so a user-selectable release channel has to
// drive check/download/install from Rust. See
// `src-tauri/src/commands/release_update.rs`.

import { invoke } from "@tauri-apps/api/core";

/** The two release channels the in-app updater can read. */
export type ReleaseChannelName = "stable" | "beta";

/** Result of a channel-aware update check — mirrors `ReleaseUpdateCheckDto`. */
export interface ReleaseUpdateCheck {
  /** Whether the manifest announced a newer version. */
  updateAvailable: boolean;
  /** Announced version (no leading `v`). `null` when up to date. */
  version: string | null;
  /** The currently-running version. Always present. */
  currentVersion: string;
  /** Release notes from the manifest. `null` when up to date / omitted. */
  notes: string | null;
  /** Publish date as `YYYY-MM-DD`, if the manifest carried one. */
  pubDate: string | null;
  /** The channel this check ran against. */
  channel: ReleaseChannelName;
  /**
   * True when the check ran on the Stable channel from a running
   * prerelease build and the stable manifest's newest version is
   * older than the running version (the Beta → Stable switch case).
   * Not an update, but not "you're on the latest version" either —
   * the UI renders a dedicated explanation.
   */
  strandedOnPrerelease: boolean;
  /** The stable manifest's version when stranded; `null` otherwise. */
  stableVersion: string | null;
}

/**
 * One download-progress tick. Emitted on the `release-update://download`
 * Tauri event during `releaseUpdateInstall`. Mirrors the Rust
 * `DownloadProgress` enum (serde-tagged on `event`).
 */
export type ReleaseDownloadProgress =
  | { event: "started"; contentLength: number | null }
  | { event: "progress"; downloaded: number; contentLength: number | null }
  | { event: "finished" };

/** Tauri event name the download progress payloads arrive on. */
export const RELEASE_DOWNLOAD_EVENT = "release-update://download";

export const releaseUpdateApi = {
  /** Read the persisted release channel preference. */
  releaseChannelGet: () => invoke<ReleaseChannelName>("release_channel_get"),

  /**
   * Persist a new release channel. Takes effect on the next
   * `releaseUpdateCheck` — no app restart needed. Returns the
   * normalized channel string.
   */
  releaseChannelSet: (channel: ReleaseChannelName) =>
    invoke<ReleaseChannelName>("release_channel_set", { channel }),

  /**
   * Run a channel-aware update check. Reads the persisted channel,
   * checks that channel's manifest, and stashes the resulting update
   * handle Rust-side for a subsequent `releaseUpdateInstall`.
   */
  releaseUpdateCheck: () =>
    invoke<ReleaseUpdateCheck>("release_update_check"),

  /**
   * Download + install the update stashed by the last successful
   * `releaseUpdateCheck`. Emits progress on `RELEASE_DOWNLOAD_EVENT`.
   * Errors if no update is staged. After this resolves, relaunch via
   * `@tauri-apps/plugin-process`.
   */
  releaseUpdateInstall: () => invoke<void>("release_update_install"),

  /**
   * Pre-relaunch quiesce probe — labels of background ops still
   * running (same busy definition as the quit gate). Call before
   * relaunching so restart-to-update can warn-confirm instead of
   * killing in-flight work.
   */
  relaunchBusyOps: () => invoke<string[]>("release_relaunch_busy_ops"),
};
