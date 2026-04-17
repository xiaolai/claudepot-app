import { useCallback, useEffect, useRef, useState } from "react";

export type Toast = {
  id: number;
  kind: "info" | "error";
  text: string;
  exiting: boolean;
  /** Optional undo callback — shown as a button on the toast. */
  onUndo?: () => void;
};

let toastCounter = 0;

export function useToasts() {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const timersRef = useRef<Map<number, ReturnType<typeof setTimeout>>>(new Map());

  // Clear all pending timers on unmount
  useEffect(() => {
    const timers = timersRef.current;
    return () => {
      for (const t of timers.values()) clearTimeout(t);
      timers.clear();
    };
  }, []);

  const removeToast = useCallback((id: number) => {
    setToasts((t) => t.filter((x) => x.id !== id));
    timersRef.current.delete(id);
  }, []);

  const dismissToast = useCallback((id: number) => {
    const timer = timersRef.current.get(id);
    if (timer) {
      clearTimeout(timer);
      timersRef.current.delete(id);
    }
    setToasts((t) => t.map((x) => (x.id === id ? { ...x, exiting: true } : x)));
    setTimeout(() => removeToast(id), 150);
  }, [removeToast]);

  const pushToast = useCallback((kind: Toast["kind"], text: string, onUndo?: () => void) => {
    toastCounter += 1;
    const id = toastCounter;
    setToasts((t) => [...t, { id, kind, text, exiting: false, onUndo }]);
    if (kind === "info") {
      const timer = setTimeout(() => dismissToast(id), onUndo ? 3000 : 4000);
      timersRef.current.set(id, timer);
    }
  }, [dismissToast]);

  return { toasts, pushToast, dismissToast };
}
