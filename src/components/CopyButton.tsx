import { useRef, useState } from "react";
import { Copy, Check, AlertTriangle } from "lucide-react";

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
      {state === "copied" ? <Check size={13} strokeWidth={2.5} /> :
       state === "failed" ? <AlertTriangle size={13} /> :
       <Copy size={13} />}
    </button>
  );
}
