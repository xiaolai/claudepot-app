import { useRef, useState } from "react";

export function CopyButton({ text }: { text: string }) {
  const [state, setState] = useState<"idle" | "copied" | "failed">("idle");
  const timerRef = useRef<ReturnType<typeof setTimeout>>();

  const copy = async () => {
    clearTimeout(timerRef.current);
    try {
      await navigator.clipboard.writeText(text);
      setState("copied");
    } catch {
      setState("failed");
    }
    timerRef.current = setTimeout(() => setState("idle"), 1500);
  };

  const label = state === "copied" ? "✓" : state === "failed" ? "!" : "⎘";

  return (
    <button className="copy-btn" onClick={copy} title="Copy to clipboard"
      aria-label={`Copy ${text}`}>
      {label}
    </button>
  );
}
