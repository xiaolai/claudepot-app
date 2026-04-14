import { useState } from "react";

export function CopyButton({ text }: { text: string }) {
  const [state, setState] = useState<"idle" | "copied" | "failed">("idle");

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setState("copied");
    } catch {
      setState("failed");
    }
    setTimeout(() => setState("idle"), 1500);
  };

  const label = state === "copied" ? "✓" : state === "failed" ? "!" : "⎘";

  return (
    <button className="copy-btn" onClick={copy} title="Copy to clipboard"
      aria-label={`Copy ${text}`}>
      {label}
    </button>
  );
}
