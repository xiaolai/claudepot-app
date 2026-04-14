import { useState } from "react";

export function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);

  const copy = async () => {
    await navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  return (
    <button className="copy-btn" onClick={copy} title="Copy to clipboard"
      aria-label={`Copy ${text}`}>
      {copied ? "✓" : "⎘"}
    </button>
  );
}
