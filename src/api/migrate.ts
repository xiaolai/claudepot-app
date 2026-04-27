// Project migrate API — thin wrappers over the Tauri commands in
// `src-tauri/src/commands_migrate.rs`. See
// `dev-docs/project-migrate-spec.md` §12.3 for the surface contract.
//
// No call carries credentials. Account stubs come back from the
// importer as part of the receipt; the user re-logs in on the target
// to actually use them.

import { invoke } from "@tauri-apps/api/core";

export type ExportArgs = {
  outputPath: string;
  projectPrefixes: string[];
  includeGlobal?: boolean;
  includeWorktree?: boolean;
  includeLive?: boolean;
  includeClaudepotState?: boolean;
  noFileHistory?: boolean;
  encrypt?: boolean;
  encryptPassphrase?: string;
  signKeyfile?: string;
  signPassword?: string;
};

export type ExportReceipt = {
  bundlePath: string;
  bundleSha256Sidecar: string;
  projectCount: number;
  fileCount: number;
};

export type ExportFlags = {
  includeGlobal: boolean;
  includeWorktree: boolean;
  includeLive: boolean;
  includeClaudepotState: boolean;
  includeFileHistory: boolean;
  encrypted: boolean;
  signed: boolean;
};

export type ProjectManifestRef = {
  id: string;
  sourceCwd: string;
  sourceSlug: string;
  sessionCount: number;
};

export type ImportPlan = {
  schemaVersion: number;
  claudepotVersion: string;
  createdAt: string;
  sourceOs: string;
  sourceArch: string;
  flags: ExportFlags;
  projects: ProjectManifestRef[];
};

export type RemapPair = { source: string; target: string };

export type ImportArgs = {
  bundlePath: string;
  mode: "skip" | "merge" | "replace";
  prefer?: "imported" | "target";
  acceptHooks?: boolean;
  acceptMcp?: boolean;
  remap?: RemapPair[];
  noFileHistory?: boolean;
  dryRun?: boolean;
  passphrase?: string;
  verifyKeyPath?: string;
};

export type AccountStub = {
  uuid: string;
  email: string;
  orgUuid: string | null;
  orgName: string | null;
  subscriptionType: string | null;
  rateLimitTier: string | null;
  verifyStatus: string;
};

export type ImportReceipt = {
  bundleId: string;
  projectsImported: string[];
  projectsRefused: [string, string][];
  journalPath: string;
  dryRun: boolean;
  accountsListed: AccountStub[];
};

export type UndoReceipt = {
  bundleId: string;
  stepsReversed: number;
  stepsTampered: string[];
  stepsErrored: string[];
  journalPath: string;
  counterJournalPath: string;
};

export const migrateApi = {
  /** Inspect a bundle's manifest. Encrypted bundles need `passphrase`. */
  inspect: (bundlePath: string, passphrase?: string) =>
    invoke<ImportPlan>("migrate_inspect", { args: { bundlePath, passphrase } }),

  /** Bundle one or more projects. */
  export: (args: ExportArgs) => invoke<ExportReceipt>("migrate_export", { args }),

  /** Import a bundle. */
  import: (args: ImportArgs) => invoke<ImportReceipt>("migrate_import", { args }),

  /** Undo the most recent import within the 24h window. */
  undo: () => invoke<UndoReceipt>("migrate_undo"),
};
