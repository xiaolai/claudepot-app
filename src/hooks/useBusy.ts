import { useState } from "react";

export function useBusy() {
  const [busyKeys, setBusyKeys] = useState<Set<string>>(new Set());

  const withBusy = async <T,>(key: string, fn: () => Promise<T>) => {
    setBusyKeys((prev) => new Set(prev).add(key));
    try {
      return await fn();
    } finally {
      setBusyKeys((prev) => {
        const next = new Set(prev);
        next.delete(key);
        return next;
      });
    }
  };

  const addBusy = (key: string) =>
    setBusyKeys((prev) => new Set(prev).add(key));

  const removeBusy = (key: string) =>
    setBusyKeys((prev) => {
      const next = new Set(prev);
      next.delete(key);
      return next;
    });

  return { busyKeys, anyBusy: busyKeys.size > 0, withBusy, addBusy, removeBusy };
}
