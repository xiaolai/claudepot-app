// The full-screen sheet both pickers wear.
//
// Two of these exist — slash commands behind `/`, quick prompts behind
// `…` — and they differ only in what a row looks like. The chrome
// (position, z-index, the filter field, the close button, the empty and
// failure states) is shared rather than copied, because a second copy is
// a second place for the fixed-position rule or the scroll container to
// drift.
import { useEffect, useMemo, useRef, useState } from 'react';

const { Ico } = window;

export function PickerSheet({
  label,
  placeholder,
  items,
  failed,
  match,
  empty,
  noMatch,
  onClose,
  children,
}) {
  const [q, setQ] = useState('');
  const filterRef = useRef(null);

  useEffect(() => {
    // The list runs to 168 entries on a real machine, so the filter is
    // the primary control, not an afterthought.
    filterRef.current?.focus();
  }, [items]);

  const shown = useMemo(() => {
    if (!items) return [];
    const needle = q.trim().toLowerCase();
    if (!needle) return items;
    return items.filter((it) => match(it, needle));
  }, [items, q, match]);

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={label}
      style={{
        // `fixed`, not `absolute`: this is a full-screen sheet, and
        // `absolute` would silently become a 100px-tall box inside the
        // composer the moment anything above it gains `position:
        // relative`. Saying viewport beats depending on no ancestor
        // ever saying otherwise.
        position: 'fixed',
        inset: 0,
        // `--z-sheet`, not a literal: `--z-bar` is also 20, and on a tie
        // the tab bar wins on DOM order and paints over this.
        zIndex: 'var(--z-sheet)',
        display: 'flex',
        flexDirection: 'column',
        background: 'var(--bg)',
        paddingTop: 'var(--safe-t)',
      }}
    >
      <div
        style={{
          display: 'flex',
          gap: 'var(--s2)',
          alignItems: 'center',
          padding: 'var(--s3) var(--gut)',
          boxShadow: 'inset 0 calc(var(--bw-hair) * -1) 0 var(--hair)',
        }}
      >
        <input
          ref={filterRef}
          value={q}
          onChange={(e) => setQ(e.target.value)}
          aria-label={placeholder}
          placeholder={placeholder}
          style={{
            flex: 1,
            height: 'var(--ctl-lg)',
            padding: '0 var(--s3)',
            borderRadius: 'var(--r-md)',
            background: 'var(--sf2)',
            color: 'var(--fg)',
            fontFamily: 'inherit',
            fontSize: 'var(--t-body)',
          }}
        />
        <button onClick={onClose} aria-label="Close" style={{ padding: 'var(--s2)' }}>
          <Ico n="x" s="sm" c="var(--fg3)" />
        </button>
      </div>

      <div
        className="sc"
        style={{ flex: 1, minHeight: 0, padding: '0 var(--gut)', paddingBottom: 'var(--safe-b)' }}
      >
        {failed && (
          <p role="alert" style={{ padding: 'var(--s4) 0', fontSize: 'var(--t-meta)', color: 'var(--dg)' }}>
            {failed}
          </p>
        )}
        {items === null && !failed && (
          <p style={{ padding: 'var(--s4) 0', fontSize: 'var(--t-meta)', color: 'var(--fg4)' }}>Loading…</p>
        )}
        {items !== null && shown.length === 0 && !failed && (
          <p style={{ padding: 'var(--s4) 0', fontSize: 'var(--t-meta)', color: 'var(--fg4)' }}>
            {items.length === 0 ? empty : noMatch}
          </p>
        )}
        {shown.map((it) => children(it))}
      </div>
    </div>
  );
}

/** The divider every picker row wears, so the two lists scan alike. */
export const PICKER_ROW_STYLE = {
  padding: 'var(--s3) 0',
  boxShadow: 'inset 0 calc(var(--bw-hair) * -1) 0 var(--hair)',
};
