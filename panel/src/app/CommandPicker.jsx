// Picking a slash command to send as text.
//
// A slash command does not survive Claude Code's peer socket: the inbox
// dispatches with `skipSlashCommands: true`, so `/audit-fix` arrives as
// nine characters of prose and Claude answers *about* it. The server
// expands one the way CC expands it itself — see `cc_commands` — and
// this is where you choose which.
//
// It **stages**, it does not send. That was a deliberate choice over a
// one-tap run: these bodies are thousands of words of instruction, some
// of them dispatch subprocesses, and the last thing between a mistyped
// filter and a 14,000-word instruction arriving in a live session
// should be a deliberate press of Send.
import { useEffect, useMemo, useRef, useState } from 'react';

import { api } from './api.js';

const { Chip, Ico } = window;

/** `14208` → `14.2k`. A body length is a warning, not an accountancy. */
function size(chars) {
  if (chars < 1000) return `${chars}`;
  return `${(chars / 1000).toFixed(1)}k`;
}

function sourceLabel(source) {
  if (source.kind === 'plugin') return source.plugin;
  return source.kind;
}

export function CommandPicker({ sessionId, onStage, onClose }) {
  const [all, setAll] = useState(null);
  const [failed, setFailed] = useState(null);
  const [q, setQ] = useState('');
  const [chosen, setChosen] = useState(null);
  const [args, setArgs] = useState('');
  const [busy, setBusy] = useState(false);
  const filterRef = useRef(null);

  useEffect(() => {
    const ctrl = new AbortController();
    api
      .commands(sessionId, ctrl.signal)
      .then((d) => setAll(d?.commands ?? []))
      .catch((e) => {
        if (e?.name !== 'AbortError') setFailed('Could not read this project’s commands.');
      });
    return () => ctrl.abort();
  }, [sessionId]);

  useEffect(() => {
    // The list runs to 168 entries on a real machine, so the filter is
    // the primary control, not an afterthought.
    filterRef.current?.focus();
  }, [all]);

  const shown = useMemo(() => {
    if (!all) return [];
    const needle = q.trim().toLowerCase();
    if (!needle) return all;
    return all.filter(
      (c) =>
        c.name.toLowerCase().includes(needle) ||
        (c.description || '').toLowerCase().includes(needle),
    );
  }, [all, q]);

  const stage = async (cmd, withArgs) => {
    setBusy(true);
    try {
      const d = await api.expandCommand(sessionId, cmd.name, withArgs);
      onStage({ name: d.name, text: d.text, chars: d.text.length, restricts_tools: d.restricts_tools });
      onClose();
    } catch {
      setFailed('Could not expand that command.');
      setBusy(false);
    }
  };

  const pick = (cmd) => {
    // Two steps only when the command actually takes arguments.
    if (cmd.argument_hint) {
      setChosen(cmd);
      setArgs('');
    } else {
      stage(cmd, '');
    }
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Insert a command"
      style={{
        // `fixed`, not `absolute`: this is a full-screen sheet, and
        // `absolute` would silently become a 100px-tall box inside the
        // composer the moment anything above it gains `position:
        // relative`. Saying viewport is better than depending on no
        // ancestor ever saying otherwise.
        position: 'fixed',
        inset: 0,
        // `--z-sheet`, not a literal: `--z-bar` is also 20, and on a
        // tie the tab bar wins on DOM order and paints over this.
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
          aria-label="Filter commands"
          placeholder="Filter commands…"
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

      <div className="sc" style={{ flex: 1, minHeight: 0, padding: '0 var(--gut)', paddingBottom: 'var(--safe-b)' }}>
        {failed && (
          <p role="alert" style={{ padding: 'var(--s4) 0', fontSize: 'var(--t-meta)', color: 'var(--dg)' }}>
            {failed}
          </p>
        )}
        {all === null && !failed && (
          <p style={{ padding: 'var(--s4) 0', fontSize: 'var(--t-meta)', color: 'var(--fg4)' }}>Loading…</p>
        )}
        {all !== null && shown.length === 0 && (
          <p style={{ padding: 'var(--s4) 0', fontSize: 'var(--t-meta)', color: 'var(--fg4)' }}>
            {all.length === 0 ? 'No commands in this project.' : 'Nothing matches that.'}
          </p>
        )}

        {shown.map((c) => {
          const open = chosen?.name === c.name;
          return (
            <div
              key={c.name}
              style={{ padding: 'var(--s3) 0', boxShadow: 'inset 0 calc(var(--bw-hair) * -1) 0 var(--hair)' }}
            >
              <button
                onClick={() => pick(c)}
                disabled={busy}
                style={{ display: 'block', width: '100%', textAlign: 'left' }}
              >
                <div style={{ display: 'flex', alignItems: 'baseline', gap: 'var(--s2)' }}>
                  <span className="mono" style={{ fontSize: 'var(--t-sub)', fontWeight: 'var(--w-semi)' }}>
                    /{c.name}
                  </span>
                  <span style={{ marginLeft: 'auto', fontSize: 'var(--t-nano)', color: 'var(--fg4)' }}>
                    {sourceLabel(c.source)} · {size(c.body_chars)}
                  </span>
                </div>
                {c.description && (
                  <div
                    style={{
                      fontSize: 'var(--t-micro)',
                      color: 'var(--fg3)',
                      marginTop: 'var(--s1)',
                      lineHeight: 'var(--lh-body)',
                    }}
                  >
                    {c.description}
                  </div>
                )}
                {/* Stated on the row, not buried on the result: invoked
                    properly this command runs under a restricted tool
                    set, and sent as text it runs under the session's. */}
                {c.restricts_tools && (
                  <div style={{ fontSize: 'var(--t-nano)', color: 'var(--wn)', marginTop: 'var(--s1)' }}>
                    <Ico n="alert" s="2xs" w="bold" /> normally limits its own tools — as text it won’t
                  </div>
                )}
              </button>

              {open && (
                <div style={{ display: 'flex', gap: 'var(--s2)', marginTop: 'var(--s2)' }}>
                  <input
                    value={args}
                    onChange={(e) => setArgs(e.target.value)}
                    aria-label={`Arguments for ${c.name}`}
                    placeholder={c.argument_hint}
                    autoFocus
                    style={{
                      flex: 1,
                      height: 'var(--ctl-md)',
                      padding: '0 var(--s3)',
                      borderRadius: 'var(--r-md)',
                      background: 'var(--sf2)',
                      color: 'var(--fg)',
                      fontFamily: 'var(--f-mono)',
                      fontSize: 'var(--t-body)',
                    }}
                  />
                  <Chip tone="accent" size="md" onClick={busy ? undefined : () => stage(c, args)}>
                    {busy ? '…' : 'Insert'}
                  </Chip>
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
