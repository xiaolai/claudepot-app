import { useCallback, useState } from "react";

export type Toast = { id: number; kind: "info" | "error"; text: string };

let toastCounter = 0;

export function useToasts() {
  const [toasts, setToasts] = useState<Toast[]>([]);

  const pushToast = useCallback((kind: Toast["kind"], text: string) => {
    toastCounter += 1;
    const id = toastCounter;
    setToasts((t) => [...t, { id, kind, text }]);
    if (kind === "info") {
      setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 4000);
    }
  }, []);

  const dismissToast = useCallback((id: number) => {
    setToasts((t) => t.filter((x) => x.id !== id));
  }, []);

  return { toasts, pushToast, dismissToast };
}
