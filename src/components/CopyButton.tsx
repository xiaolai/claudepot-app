import { useRef, useState } from "react";
import { Icon } from "./Icon";

export function CopyButton({ text }: { text: string }) {
  const [state, setState] = useState<"idle" | "copied" | "failed">("idle");
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined);

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
      aria-label={`Copy ${text}`}>
      {state === "copied" ? <Icon name="check" size={13} /> :
       state === "failed" ? <Icon name="alert-triangle" size={13} /> :
       <Icon name="copy" size={13} />}
    </button>
  );
}
