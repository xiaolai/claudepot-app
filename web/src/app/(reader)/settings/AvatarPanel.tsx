"use client";

import Image from "next/image";
import { useActionState, useId, useRef, useState } from "react";

import {
  clearAvatarAction,
  setAvatarFormAction,
  type SetAvatarFormState,
} from "@/lib/actions/avatar";

const INIT: SetAvatarFormState = { phase: "idle" };

interface Props {
  /** Current avatar URL (users.image OR users.avatarUrl resolved
   *  upstream). null = no avatar set. */
  currentUrl: string | null;
  username: string;
}

/**
 * /settings avatar editor. File-input + preview. Uploads via
 * server action, which calls lib/avatars.ts:setAvatar (validation
 * + Vercel Blob upload + DB write). The same lib helper backs the
 * REST endpoint at /api/v1/users/me/avatar.
 *
 * Intentional limits surfaced in the UI:
 *   - 2 MB max
 *   - PNG / JPEG / WebP only (SVG rejected; magic-byte verified)
 */
export function AvatarPanel({ currentUrl, username }: Props) {
  const [state, formAction, pending] = useActionState(
    setAvatarFormAction,
    INIT,
  );
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [clearPending, setClearPending] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const fieldId = useId();

  // The success state's avatarUrl wins over the initial currentUrl
  // so the panel updates without a full page reload after upload.
  const displayedUrl =
    previewUrl ?? (state.phase === "ok" ? state.avatarUrl : currentUrl);

  return (
    <div className="proto-form" aria-labelledby={fieldId}>
      <div className="proto-avatar-panel-row">
        {displayedUrl ? (
          <Image
            src={displayedUrl}
            alt={`${username}'s avatar`}
            width={96}
            height={96}
            className="proto-avatar-panel-preview"
            unoptimized
          />
        ) : (
          <div
            className="proto-avatar-panel-preview proto-avatar-panel-empty"
            aria-label="No avatar set"
          />
        )}
        <div className="proto-avatar-panel-actions">
          <form action={formAction} encType="multipart/form-data">
            <label htmlFor={fieldId} className="proto-avatar-panel-label">
              Pick image
              <input
                ref={inputRef}
                id={fieldId}
                name="avatar"
                type="file"
                accept="image/png,image/jpeg,image/webp"
                onChange={(e) => {
                  const f = e.target.files?.[0];
                  if (!f) {
                    setPreviewUrl(null);
                    return;
                  }
                  // Local preview before upload — revoked when this
                  // component unmounts or another file is picked.
                  if (previewUrl) URL.revokeObjectURL(previewUrl);
                  setPreviewUrl(URL.createObjectURL(f));
                }}
              />
            </label>
            <button type="submit" disabled={pending} className="btn primary">
              {pending ? "Uploading…" : "Upload"}
            </button>
          </form>
          {currentUrl ? (
            <button
              type="button"
              disabled={clearPending}
              onClick={async () => {
                setClearPending(true);
                try {
                  await clearAvatarAction();
                  setPreviewUrl(null);
                  if (inputRef.current) inputRef.current.value = "";
                } finally {
                  setClearPending(false);
                }
              }}
              className="btn"
            >
              {clearPending ? "Clearing…" : "Clear avatar"}
            </button>
          ) : null}
        </div>
      </div>
      <p className="proto-fineprint">
        PNG, JPEG, or WebP. Max 2 MB. SVG isn&rsquo;t accepted from the web
        UI.
      </p>
      {state.phase === "ok" ? (
        <p className="proto-form-success" role="status">
          Avatar updated.
        </p>
      ) : null}
      {state.phase === "error" ? (
        <p className="proto-form-error" role="alert">
          {state.message}
        </p>
      ) : null}
    </div>
  );
}
