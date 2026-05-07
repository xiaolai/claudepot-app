"use client";

import { useState } from "react";

/**
 * Tiny client island for the "Copy" action on the reveal page.
 *
 * The plaintext is passed as a prop (so this DOES enter the React
 * client heap once for this single page render). Tradeoff vs the
 * old useActionState flow: previously the plaintext rode through
 * the entire mint form's React state lifetime; now it lives only
 * within this leaf component until the user navigates away. The
 * outer reveal page is `force-dynamic` (no SSG, no BFCache stash),
 * so navigating away unmounts the component and the value becomes
 * GC-eligible. We could move copy to a clipboard polyfill that
 * reads the DOM `<code>` text directly, but the prop-pass keeps
 * the tree predictable.
 */
export function CopyButton({ plaintext }: { plaintext: string }) {
  const [copied, setCopied] = useState(false);

  async function copy() {
    try {
      await navigator.clipboard.writeText(plaintext);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch {
      setCopied(false);
    }
  }

  return (
    <button type="button" className="proto-btn-primary" onClick={copy}>
      {copied ? "Copied" : "Copy"}
    </button>
  );
}
