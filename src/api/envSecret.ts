// .env secret-movement — frontend bindings for the `env_vault_*` and
// `env_file_*` Tauri commands. See `src-tauri/src/commands/env_secret.rs`
// and `dev-docs/permission-and-env-secrets.md`.

import { invoke } from "@tauri-apps/api/core";
import type { KeyCopyReceiptDto } from "../types";

/** One named secret in the local vault. The plaintext value never
 *  crosses this boundary — only the non-reversible preview. */
export interface VaultSecret {
  name: string;
  /** e.g. `sk-a…cdef`, or `••••` for short secrets. */
  secretPreview: string;
  createdAtMs: number;
  updatedAtMs: number;
}

/** One key inside a project `.env*` file. The third state — *absent*
 *  — is simply the key not appearing in the file's entry list. */
export interface EnvFileEntry {
  key: string;
  state: "active" | "commented";
  /** Non-reversible preview of the value; never the value itself. */
  valuePreview: string;
}

/** One `.env*` file in a project root. */
export interface EnvFileView {
  /** Bare filename (`.env`, `.env.local`, …) — the handle the
   *  mutation commands take. */
  fileName: string;
  /** Absolute path, for display + copy. */
  path: string;
  entries: EnvFileEntry[];
}

/** Every `.env*` file found in a project root. */
export interface ProjectEnv {
  projectPath: string;
  files: EnvFileView[];
}

export const envSecretApi = {
  // Vault
  envVaultList: () => invoke<VaultSecret[]>("env_vault_list"),
  envVaultAdd: (name: string, secret: string) =>
    invoke<VaultSecret>("env_vault_add", { name, secret }),
  envVaultUpdate: (name: string, secret: string) =>
    invoke<VaultSecret>("env_vault_update", { name, secret }),
  envVaultDelete: (name: string) =>
    invoke<void>("env_vault_delete", { name }),
  envVaultCopy: (name: string) =>
    invoke<KeyCopyReceiptDto>("env_vault_copy", { name }),

  // Per-project .env files
  envFileList: (projectPath: string) =>
    invoke<ProjectEnv>("env_file_list", { projectPath }),
  envFileSet: (
    projectPath: string,
    fileName: string,
    key: string,
    value: string,
  ) =>
    invoke<ProjectEnv>("env_file_set", {
      projectPath,
      fileName,
      key,
      value,
    }),
  envFileComment: (projectPath: string, fileName: string, key: string) =>
    invoke<ProjectEnv>("env_file_comment", { projectPath, fileName, key }),
  envFileUncomment: (projectPath: string, fileName: string, key: string) =>
    invoke<ProjectEnv>("env_file_uncomment", { projectPath, fileName, key }),
  envFileDelete: (projectPath: string, fileName: string, key: string) =>
    invoke<ProjectEnv>("env_file_delete", { projectPath, fileName, key }),
  envFileCopyValue: (projectPath: string, fileName: string, key: string) =>
    invoke<KeyCopyReceiptDto>("env_file_copy_value", {
      projectPath,
      fileName,
      key,
    }),
  envFileInject: (
    projectPath: string,
    fileName: string,
    vaultName: string,
  ) =>
    invoke<ProjectEnv>("env_file_inject", {
      projectPath,
      fileName,
      vaultName,
    }),
};
