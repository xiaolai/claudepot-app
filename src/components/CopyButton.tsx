import { useRef, useState } from "react";
import { Icon } from "./Icon";

/**
 * One-click clipboard copy with a 1.5s success/fail glyph swap.
 *
 * `ariaLabel` overrides the screen-reader label. Default is
 * `Copy ${text}` which reads cleanly for short identifiers (paths,
 * UUIDs) but is broken for prose — a 4000-char message body should
 * never be read aloud verbatim. Pass a short descriptive label
 * like `"Copy assistant message"` for body-text surfaces.
 */
export function CopyButton({
  text,
  ariaLabel,
}: {
  text: string;
  ariaLabel?: string;
}) {
  const [state, setState] = useState<"idle" | "copied" | "failed">("idle");
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  // Render nothing for empty payloads. Otherwise the user clicks a
  // button expecting to copy a bubble's content and instead overwrites
  // whatever they had on the clipboard with `""`. Every call site
  // benefits — guard centrally rather than at each one.
  if (text.length === 0) return null;

  const copy = async () => {
    clearTimeout(timerRef.current);
    try {
      await navigator.clipboard.writeText(text);
      setState("copied");
    } catch (_e) {
      setState("failed");
    }
    timerRef.current = setTimeout(() => setState("idle"), 1500);
  };

  return (
    <button className="copy-btn" onClick={copy} title="Copy to clipboard"
      aria-label={ariaLabel ?? `Copy ${text}`}>
      {state === "copied" ? <Icon name="check" size={13} /> :
       state === "failed" ? <Icon name="alert-triangle" size={13} /> :
       <Icon name="copy" size={13} />}
    </button>
  );
}
