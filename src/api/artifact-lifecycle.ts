// Tauri bridge for the 9 artifact-lifecycle commands.
//
// Read-only:  artifactClassifyPath, artifactListDisabled, artifactListTrash
// Mutating:   artifactDisable, artifactEnable, artifactTrash,
//             artifactRestoreFromTrash, artifactRecoverTrash,
//             artifactForgetTrash, artifactPurgeTrash

import { invoke } from "@tauri-apps/api/core";
import type {
  ClassifyPathDto,
  DisabledRecordDto,
  LifecycleKind,
  OnConflict,
  RestoredArtifactDto,
  TrashEntryDto,
} from "../types";

export const artifactLifecycleApi = {
  artifactClassifyPath: (absPath: string, projectRoot: string | null) =>
    invoke<ClassifyPathDto>("artifact_classify_path", {
      absPath,
      projectRoot,
    }),

  artifactDisable: (
    scopeRoot: string,
    kind: LifecycleKind,
    relativePath: string,
    onConflict: OnConflict,
    projectRoot: string | null,
  ) =>
    invoke<DisabledRecordDto>("artifact_disable", {
      scopeRoot,
      kind,
      relativePath,
      onConflict,
      projectRoot,
    }),

  artifactEnable: (
    scopeRoot: string,
    kind: LifecycleKind,
    relativePath: string,
    onConflict: OnConflict,
    projectRoot: string | null,
  ) =>
    invoke<DisabledRecordDto>("artifact_enable", {
      scopeRoot,
      kind,
      relativePath,
      onConflict,
      projectRoot,
    }),

  artifactListDisabled: (projectRoot: string | null) =>
    invoke<DisabledRecordDto[]>("artifact_list_disabled", { projectRoot }),

  artifactTrash: (
    scopeRoot: string,
    kind: LifecycleKind,
    relativePath: string,
    projectRoot: string | null,
  ) =>
    invoke<TrashEntryDto>("artifact_trash", {
      scopeRoot,
      kind,
      relativePath,
      projectRoot,
    }),

  artifactListTrash: () => invoke<TrashEntryDto[]>("artifact_list_trash"),

  artifactRestoreFromTrash: (trashId: string, onConflict: OnConflict) =>
    invoke<RestoredArtifactDto>("artifact_restore_from_trash", {
      trashId,
      onConflict,
    }),

  artifactRecoverTrash: (
    trashId: string,
    confirmedTargetPath: string,
    confirmedKind: LifecycleKind,
    onConflict: OnConflict,
  ) =>
    invoke<RestoredArtifactDto>("artifact_recover_trash", {
      trashId,
      confirmedTargetPath,
      confirmedKind,
      onConflict,
    }),

  artifactForgetTrash: (trashId: string) =>
    invoke<void>("artifact_forget_trash", { trashId }),

  artifactPurgeTrash: (olderThanDays: number) =>
    invoke<number>("artifact_purge_trash", { olderThanDays }),
};
