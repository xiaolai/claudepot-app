"use client";

import { useEffect } from "react";

export default function ErrorBoundary({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  useEffect(() => {
    console.error(error);
  }, [error]);

  return (
    <div className="proto-page-narrow">
      <h1>Something broke</h1>
      <p className="proto-dek">
        An unexpected error occurred while rendering this page.
        {error.digest ? (
          <>
            {" "}
            Reference: <code>{error.digest}</code>
          </>
        ) : null}
      </p>
      <p>
        <button type="button" className="proto-btn-primary" onClick={reset}>
          Try again
        </button>
      </p>
    </div>
  );
}
