import React from "react";

/**
 * Local error boundary for the virtualized session list. The
 * virtualizer can, in rare cases, throw during its measurement pass —
 * a bad `ResizeObserver` callback, a disconnected node, a `NaN`
 * height. Without this boundary the error bubbles to the top-level
 * `ErrorBoundary` and blanks the whole app window. We catch it here
 * and drop back to the plain list, losing virtualization but
 * preserving functionality.
 *
 * `resetKey` lets the boundary recover. A measurement glitch is often
 * tied to one specific dataset shape; once `sessions` changes (filter,
 * sort, refresh) we get another shot at virtualization. Without this,
 * a single transient error would latch the section to PlainList until
 * the user navigates away.
 */
interface VirtualFallbackProps {
  children: React.ReactNode;
  fallback: React.ReactNode;
  /** Bumping this resets the failed state so the next paint can retry
   * the virtualized path. Pass something tied to the dataset shape. */
  resetKey: unknown;
}

interface VirtualFallbackState {
  failed: boolean;
  resetKey: unknown;
}

export class VirtualFallbackBoundary extends React.Component<
  VirtualFallbackProps,
  VirtualFallbackState
> {
  state: VirtualFallbackState = {
    failed: false,
    resetKey: this.props.resetKey,
  };

  static getDerivedStateFromError(): Pick<VirtualFallbackState, "failed"> {
    return { failed: true };
  }

  static getDerivedStateFromProps(
    props: VirtualFallbackProps,
    state: VirtualFallbackState,
  ): Partial<VirtualFallbackState> | null {
    if (props.resetKey !== state.resetKey) {
      return { failed: false, resetKey: props.resetKey };
    }
    return null;
  }

  componentDidCatch(err: unknown): void {
    if (import.meta.env.DEV) {
      // eslint-disable-next-line no-console
      console.error(
        "[SessionsTable] virtualizer threw — falling back to PlainList",
        err,
      );
    }
  }

  render(): React.ReactNode {
    return this.state.failed ? this.props.fallback : this.props.children;
  }
}
