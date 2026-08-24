// The two things every screen needs and neither should own.
//
// `Muted` was copied into Projects and Accounts, differing only by
// padding — the loading/error/empty line is the same primitive on every
// screen, and two copies is how the third one ends up saying something
// slightly different.
//
// `CopyPath` exists because `.claude/rules/path-display.md` requires it:
// a truncated path with no canonical copy site elsewhere is State C, and
// State C needs both a tooltip and a copy affordance. Every path this
// panel shows is truncated and there is no detail view to copy from.
import { useEffect, useRef, useState } from 'react';

const { Ico } = window;

export function Muted({ children }) {
  return (
    <p
      role="status"
      style={{ padding: 'var(--s5) 0', color: 'var(--fg4)', fontSize: 'var(--t-meta)' }}
    >
      {children}
    </p>
  );
}

/**
 * Copy a filesystem path.
 *
 * `navigator.clipboard` is the right API here and only here: a path is
 * not a secret, and `rules/architecture.md` carves this exact case out
 * of the Rust-side clipboard rule. A secret would have to travel a
 * different way — nothing on this surface does.
 */
export function CopyPath({ path, label = 'Copy path' }) {
  const [done, setDone] = useState(false);
  // Copy, then navigate away inside the confirmation window, and the
  // timer would set state on a component that no longer exists.
  const timer = useRef(null);
  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    },
    [],
  );
  if (!path) return null;
  const copy = async (e) => {
    e.stopPropagation();
    try {
      await navigator.clipboard.writeText(path);
      setDone(true);
      if (timer.current !== null) window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => {
        timer.current = null;
        setDone(false);
      }, 1600);
    } catch {
      // Safari refuses outside a user gesture and some contexts have no
      // clipboard at all. Silent is right: the path is still selectable,
      // and an error toast for a copy nobody watched is noise.
    }
  };
  return (
    <button
      onClick={copy}
      aria-label={label}
      title={path}
      style={{
        width: 'var(--tap-xs)',
        height: 'var(--tap-xs)',
        display: 'grid',
        placeItems: 'center',
        borderRadius: 'var(--r-pill)',
        color: done ? 'var(--ok)' : 'var(--fg4)',
        flexShrink: 0,
      }}
    >
      <Ico n={done ? 'check' : 'file'} s="sm" w="reg" />
    </button>
  );
}
