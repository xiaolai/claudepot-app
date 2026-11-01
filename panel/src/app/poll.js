// One request at a time, for something that asks on a timer.
//
// The poll used to abort the request before it on every tick, so a host
// slower than the interval was never heard from, and since an abort is not a
// failure the panel went on saying it was online. A timer has no reason to
// think its answer is better than the one already on its way, so a tick now
// waits. An abort is still right when something has just changed, because
// then the answer on its way describes the world before the change: that is
// `now`.
//
// What stops a tick waiting forever is the read deadline in `api.js`.
//
// Every call resolves and none rejects: `dropped` means nobody wants this
// answer, whether it never started, was replaced or was abandoned. The check
// is on the slot rather than on the error because `fetch` rejects
// asynchronously, and a replaced request can already have succeeded.
export function requestSlot() {
  let current = null;

  const start = (run) => {
    const ctrl = new AbortController();
    current = ctrl;
    const settle = (result) => {
      if (current !== ctrl) return { status: 'dropped' };
      current = null;
      return result;
    };
    return run(ctrl.signal).then(
      (value) => settle({ status: 'fulfilled', value }),
      (reason) => settle({ status: 'rejected', reason }),
    );
  };

  return {
    /** Ask unless a question is already out. */
    tick: (run) => (current ? Promise.resolve({ status: 'dropped' }) : start(run)),
    /** Ask now, abandoning any question already out. */
    now: (run) => {
      current?.abort();
      return start(run);
    },
    stop: () => {
      current?.abort();
      current = null;
    },
  };
}
