"use client";

import Image from "next/image";
import {
  type FormEvent,
  useActionState,
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
} from "react";
import Cropper, { type Area } from "react-easy-crop";

import {
  clearAvatarAction,
  setAvatarFormAction,
  type SetAvatarFormState,
} from "@/lib/actions/avatar";

const INIT: SetAvatarFormState = { phase: "idle" };

/** Output cap for the cropped avatar. 512×512 is more than any
 *  display surface needs (the largest render is the /settings preview
 *  at 96×96, double-density screens want 192×192) and meaningfully
 *  smaller than the 2 MB upload cap once re-encoded. Source images
 *  smaller than this stay at their native size — no upscaling. */
const OUTPUT_MAX = 512;

/** Output content-type per source content-type. PNG keeps transparency;
 *  JPEG re-encodes; WebP stays WebP. canvas.toBlob accepts these
 *  three; anything else falls back to PNG. */
function outputType(sourceType: string): "image/png" | "image/jpeg" | "image/webp" {
  if (sourceType === "image/jpeg") return "image/jpeg";
  if (sourceType === "image/webp") return "image/webp";
  return "image/png";
}

interface Props {
  /** Current avatar URL (users.image OR users.avatarUrl resolved
   *  upstream). null = no avatar set. */
  currentUrl: string | null;
  username: string;
}

/**
 * /settings avatar editor with client-side crop + downscale.
 *
 * Flow:
 *   1. User picks a file → object URL drives the Cropper.
 *   2. User pans/zooms to position the square crop window.
 *   3. On Upload: extract the crop region to a canvas, downscale to
 *      OUTPUT_MAX, re-encode as the same MIME as the source (PNG /
 *      JPEG / WebP), submit as a File via the same server action that
 *      backed the previous as-is upload.
 *
 * Server-side validation (lib/avatars.ts) is unchanged — it still
 * verifies size + magic bytes. The client cropper just guarantees
 * the bytes the server receives are square + reasonably sized.
 */
export function AvatarPanel({ currentUrl, username }: Props) {
  const [state, formAction, pending] = useActionState(
    setAvatarFormAction,
    INIT,
  );
  const [sourceUrl, setSourceUrl] = useState<string | null>(null);
  const [sourceType, setSourceType] = useState<string | null>(null);
  const [crop, setCrop] = useState({ x: 0, y: 0 });
  const [zoom, setZoom] = useState(1);
  const [pixelCrop, setPixelCrop] = useState<Area | null>(null);
  const [clearPending, setClearPending] = useState(false);
  const [croppingPending, setCroppingPending] = useState(false);
  const [clientError, setClientError] = useState<string | null>(null);
  const formRef = useRef<HTMLFormElement>(null);
  const fieldId = useId();

  // Revoke any in-flight object URL on unmount so we don't leak GPU
  // memory for large source images. (We also revoke explicitly on
  // reset() so picking another file in the same session is clean.)
  useEffect(() => {
    return () => {
      if (sourceUrl) URL.revokeObjectURL(sourceUrl);
    };
    // sourceUrl in deps would revoke before the cropper sees it;
    // unmount cleanup is the right scope.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const onCropComplete = useCallback((_area: Area, areaPixels: Area) => {
    setPixelCrop(areaPixels);
  }, []);

  function reset() {
    if (sourceUrl) URL.revokeObjectURL(sourceUrl);
    setSourceUrl(null);
    setSourceType(null);
    setCrop({ x: 0, y: 0 });
    setZoom(1);
    setPixelCrop(null);
    setClientError(null);
  }

  async function handleSubmit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    if (!sourceUrl || !sourceType || !pixelCrop) {
      setClientError("Pick an image and adjust the crop window first.");
      return;
    }
    setCroppingPending(true);
    setClientError(null);
    try {
      const blob = await renderCroppedBlob(
        sourceUrl,
        pixelCrop,
        outputType(sourceType),
      );
      const ext = outputType(sourceType).split("/")[1];
      const croppedFile = new File([blob], `avatar.${ext}`, {
        type: outputType(sourceType),
      });
      const fd = new FormData();
      fd.set("avatar", croppedFile);
      // useActionState's bound dispatcher accepts a FormData payload.
      formAction(fd);
    } catch (err) {
      setClientError(
        err instanceof Error
          ? err.message
          : "Crop failed; try a smaller image.",
      );
    } finally {
      setCroppingPending(false);
    }
  }

  // After a successful upload drop the cropper UI back to the
  // resting state so the new avatar shows in the preview.
  useEffect(() => {
    if (state.phase === "ok" && sourceUrl) reset();
    // sourceUrl deliberately not in deps — we only want to react to
    // a fresh server-action result.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state]);

  const displayedUrl =
    state.phase === "ok" ? state.avatarUrl : currentUrl;

  return (
    <div className="proto-form" aria-labelledby={fieldId}>
      {!sourceUrl ? (
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
            <label htmlFor={fieldId} className="btn">
              Pick image
            </label>
            <input
              id={fieldId}
              type="file"
              accept="image/png,image/jpeg,image/webp"
              hidden
              onChange={(e) => {
                const f = e.target.files?.[0];
                if (!f) return;
                setSourceType(f.type);
                setSourceUrl(URL.createObjectURL(f));
                // Reset the file input so picking the same file again
                // re-fires onChange.
                e.target.value = "";
              }}
            />
            {currentUrl ? (
              <button
                type="button"
                disabled={clearPending}
                onClick={async () => {
                  setClearPending(true);
                  try {
                    await clearAvatarAction();
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
      ) : (
        <form
          ref={formRef}
          onSubmit={handleSubmit}
          className="proto-avatar-cropper-form"
        >
          <div className="proto-avatar-cropper">
            <Cropper
              image={sourceUrl}
              crop={crop}
              zoom={zoom}
              aspect={1}
              showGrid={false}
              cropShape="rect"
              onCropChange={setCrop}
              onZoomChange={setZoom}
              onCropComplete={onCropComplete}
            />
          </div>
          <div className="proto-avatar-cropper-controls">
            <label className="proto-avatar-cropper-zoom">
              Zoom
              <input
                type="range"
                min={1}
                max={3}
                step={0.05}
                value={zoom}
                onChange={(e) => setZoom(Number(e.target.value))}
              />
            </label>
            <div className="proto-avatar-cropper-buttons">
              <button
                type="button"
                onClick={reset}
                className="btn"
                disabled={pending || croppingPending}
              >
                Cancel
              </button>
              <button
                type="submit"
                disabled={pending || croppingPending || !pixelCrop}
                className="btn primary"
              >
                {croppingPending
                  ? "Cropping…"
                  : pending
                    ? "Uploading…"
                    : "Upload"}
              </button>
            </div>
          </div>
        </form>
      )}
      <p className="proto-fineprint">
        PNG, JPEG, or WebP. Max 2 MB. Drag and zoom to choose what fits
        in the square. SVG isn&rsquo;t accepted from the web UI.
      </p>
      {clientError ? (
        <p className="proto-form-error" role="alert">
          {clientError}
        </p>
      ) : null}
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

/* ── Crop → blob ─────────────────────────────────────────────── */

async function renderCroppedBlob(
  sourceUrl: string,
  pixelCrop: Area,
  type: "image/png" | "image/jpeg" | "image/webp",
): Promise<Blob> {
  const image = await loadImage(sourceUrl);

  // Downscale to OUTPUT_MAX so a 4000×4000 source becomes a 512×512
  // avatar. Sources smaller than the cap stay at their native size —
  // no upscaling artifacts. Both sides of the canvas are equal because
  // the crop window's aspect ratio is pinned to 1 in the Cropper.
  const targetSize = Math.min(OUTPUT_MAX, pixelCrop.width, pixelCrop.height);

  const canvas = document.createElement("canvas");
  canvas.width = targetSize;
  canvas.height = targetSize;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Canvas 2D context unavailable.");
  ctx.imageSmoothingEnabled = true;
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(
    image,
    pixelCrop.x,
    pixelCrop.y,
    pixelCrop.width,
    pixelCrop.height,
    0,
    0,
    targetSize,
    targetSize,
  );

  const quality = type === "image/png" ? undefined : 0.9;
  return new Promise<Blob>((resolve, reject) => {
    canvas.toBlob(
      (blob) => {
        if (!blob) reject(new Error("Canvas export returned null."));
        else resolve(blob);
      },
      type,
      quality,
    );
  });
}

function loadImage(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new window.Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error("Failed to decode the image."));
    img.src = url;
  });
}
