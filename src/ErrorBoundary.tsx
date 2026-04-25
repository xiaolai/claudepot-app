import React from "react";
import { redactSecrets } from "./lib/redactSecrets";

interface State {
  /** User-visible message — always pre-redacted and length-bounded. */
  error: string | null;
}

/** Hard cap on what we render into the DOM. Long pasted prompts and
 *  multi-MB stack-strings could otherwise blow past the available
 *  space and freeze the UI. 200 chars is well past the useful signal
 *  ("ReferenceError: foo is not defined") for a fallback panel — the
 *  full sanitized error still goes to the console for diagnostics. */
const MAX_USER_VISIBLE = 200;

function sanitizeForDom(err: unknown): string {
  const raw = err instanceof Error ? err.message : String(err);
  // Audit T4-10: tokens leaking through props or thrown error messages
  // would otherwise appear verbatim in the fallback panel. Run the
  // same redactor the toast layer uses, then truncate.
  const redacted = redactSecrets(raw);
  if (redacted.length <= MAX_USER_VISIBLE) return redacted;
  return `${redacted.slice(0, MAX_USER_VISIBLE - 1)}…`;
}

export class ErrorBoundary extends React.Component<
  { children: React.ReactNode },
  State
> {
  state: State = { error: null };

  static getDerivedStateFromError(err: unknown): State {
    return { error: sanitizeForDom(err) };
  }

  /**
   * Audit T4-9: previously the boundary rendered a fallback but never
   * logged anything, so React warnings about the underlying crash
   * disappeared from the devtools console. Wire `componentDidCatch`
   * so a single console line carries the redacted error + the
   * component stack — enough to grep for in support, not enough to
   * surface a token to a screen-watcher.
   */
  componentDidCatch(error: unknown, info: React.ErrorInfo): void {
    const message = sanitizeForDom(error);
    const componentStack = info.componentStack ?? "";
    // eslint-disable-next-line no-console
    console.error(
      "[ErrorBoundary] caught render-phase error:",
      message,
      componentStack,
    );
  }

  render() {
    if (this.state.error) {
      return (
        <main className="app loading">
          <div className="empty">
            <h2>Something went wrong</h2>
            <p className="muted mono">{this.state.error}</p>
            <button
              className="btn primary"
              onClick={() => window.location.reload()}
            >
              Retry
            </button>
          </div>
        </main>
      );
    }
    return this.props.children;
  }
}
