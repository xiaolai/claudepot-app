import { useCallback, useRef, useState } from "react";

export type Toast = { id: number; kind: "info" | "error"; text: string; exiting: boolean };

let toastCounter = 0;

export function useToasts() {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const timersRef = useRef<Map<number, ReturnType<typeof setTimeout>>>(new Map());

  const removeToast = useCallback((id: number) => {
    setToasts((t) => t.filter((x) => x.id !== id));
    timersRef.current.delete(id);
  }, []);

  const dismissToast = useCallback((id: number) => {
    // Clear any pending auto-dismiss timer
    const timer = timersRef.current.get(id);
    if (timer) {
      clearTimeout(timer);
      timersRef.current.delete(id);
    }
    // Mark as exiting, then remove after animation
    setToasts((t) => t.map((x) => (x.id === id ? { ...x, exiting: true } : x)));
    setTimeout(() => removeToast(id), 150);
  }, [removeToast]);

  const pushToast = useCallback((kind: Toast["kind"], text: string) => {
    toastCounter += 1;
    const id = toastCounter;
    setToasts((t) => [...t, { id, kind, text, exiting: false }]);
    if (kind === "info") {
      const timer = setTimeout(() => dismissToast(id), 4000);
      timersRef.current.set(id, timer);
    }
  }, [dismissToast]);

  return { toasts, pushToast, dismissToast };
}
