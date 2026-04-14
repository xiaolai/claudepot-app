import { useCallback, useState } from "react";

export type Toast = { id: number; kind: "info" | "error"; text: string; exiting?: boolean };

let toastCounter = 0;

export function useToasts() {
  const [toasts, setToasts] = useState<Toast[]>([]);

  const pushToast = useCallback((kind: Toast["kind"], text: string) => {
    toastCounter += 1;
    const id = toastCounter;
    setToasts((t) => [...t, { id, kind, text }]);
    if (kind === "info") {
      setTimeout(() => dismissToast(id), 4000);
    }
  }, []);

  const dismissToast = useCallback((id: number) => {
    // Mark as exiting, then remove after animation
    setToasts((t) => t.map((x) => (x.id === id ? { ...x, exiting: true } : x)));
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 150);
  }, []);

  return { toasts, pushToast, dismissToast };
}
