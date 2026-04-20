import React from "react";

interface State {
  error: string | null;
}

export class ErrorBoundary extends React.Component<
  { children: React.ReactNode },
  State
> {
  state: State = { error: null };

  static getDerivedStateFromError(err: unknown): State {
    return { error: err instanceof Error ? err.message : String(err) };
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
