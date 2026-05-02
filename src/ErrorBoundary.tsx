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

interface ErrorBoundaryProps {
  children: React.ReactNode;
  /** When set, render a scoped in-place fallback instead of the
   *  full-app takeover. Use this to wrap individual sections so a
   *  single section crash doesn't blank the whole app. The label
   *  appears in the fallback heading and the console log. */
  label?: string;
}

export class ErrorBoundary extends React.Component<ErrorBoundaryProps, State> {
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
    const tag = this.props.label
      ? `[ErrorBoundary:${this.props.label}]`
      : "[ErrorBoundary]";
    // eslint-disable-next-line no-console
    console.error(
      `${tag} caught render-phase error:`,
      message,
      componentStack,
    );
  }

  private reset = () => {
    this.setState({ error: null });
  };

  render() {
    if (this.state.error) {
      const { label } = this.props;
      if (label) {
        // Scoped fallback: in-place, contained, with a soft "try again"
        // that re-mounts the section subtree without reloading the app.
        return (
          <div
            role="alert"
            style={{
              display: "flex",
              flexDirection: "column",
              alignItems: "center",
              justifyContent: "center",
              gap: "var(--sp-8)",
              padding: "var(--sp-32)",
              minHeight: "tokens.settings.nav.width",
              fontFamily: "var(--font)",
            }}
          >
            <h2 style={{ margin: 0, fontSize: "var(--fs-md)" }}>
              {label} couldn’t render
            </h2>
            <p
              className="mono"
              style={{
                margin: 0,
                color: "var(--fg-muted)",
                fontSize: "var(--fs-sm)",
                maxWidth: "tokens.modal.width.md",
                textAlign: "center",
              }}
            >
              {this.state.error}
            </p>
            <button className="btn" onClick={this.reset}>
              Try again
            </button>
          </div>
        );
      }
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
