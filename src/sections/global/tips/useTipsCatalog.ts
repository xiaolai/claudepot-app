// Catalog fetching hook. On mount: load + record snapshot. Refresh
// re-extracts from the binary and re-renders.

import { useCallback, useEffect, useState } from "react";
import { api } from "../../../api";
import type { TipsRender } from "../../../types/cc-tips";

interface State {
  data: TipsRender | null;
  loading: boolean;
  error: string | null;
  refreshing: boolean;
}

export function useTipsCatalog() {
  const [state, setState] = useState<State>({
    data: null,
    loading: true,
    error: null,
    refreshing: false,
  });

  const load = useCallback(async () => {
    try {
      const r = await api.ccTipsList();
      setState((s) => ({ ...s, data: r, loading: false, error: null }));
    } catch (e) {
      setState((s) => ({
        ...s,
        loading: false,
        error: e instanceof Error ? e.message : String(e),
      }));
    }
  }, []);

  const refresh = useCallback(async () => {
    setState((s) => ({ ...s, refreshing: true }));
    try {
      await api.ccTipsRefresh();
      await load();
    } catch (e) {
      setState((s) => ({
        ...s,
        error: e instanceof Error ? e.message : String(e),
      }));
    } finally {
      setState((s) => ({ ...s, refreshing: false }));
    }
  }, [load]);

  useEffect(() => {
    void load();
    // Record a snapshot once per mount; the backend debounces to 1
    // hour so spamming the tab is fine.
    void api.ccTipsRecordView().catch(() => {
      /* swallow — non-critical */
    });
  }, [load]);

  return {
    ...state,
    refresh,
  };
}
