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
import { useEffect, useRef, useState } from 'react';

import { api } from './api.js';
import { PickerSheet, PICKER_ROW_STYLE } from './PickerSheet.jsx';

const { Chip } = window;

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
  const [chosen, setChosen] = useState(null);
  const [args, setArgs] = useState('');
  const [busy, setBusy] = useState(false);

  // `AbortController` alone is not enough. A fetch that resolves in the
  // gap between the abort and the unmount still runs its `.then`, so a
  // stale command list could be applied after the sheet closed or the
  // session changed — the same generation guard `useTranscript` already
  // carries, for the same reason.
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  // The CURRENT session, in a ref.
  //
  // Comparing a captured `sessionId` against the `sessionId` from the
  // same render closure is a tautology — both are the old value, so the
  // guard always passed. A ref is the only thing an async continuation
  // can read that reflects the prop as it is NOW rather than as it was
  // when the tap happened.
  const currentSession = useRef(sessionId);
  useEffect(() => {
    currentSession.current = sessionId;
  }, [sessionId]);

  useEffect(() => {
    const ctrl = new AbortController();
    let current = true;
    api
      .commands(sessionId, ctrl.signal)
      .then((d) => {
        if (current && alive.current) setAll(d?.commands ?? []);
      })
      .catch((e) => {
        if (!current || !alive.current) return;
        if (e?.name !== 'AbortError') setFailed('Could not read this project\u2019s commands.');
      });
    return () => {
      current = false;
      ctrl.abort();
    };
  }, [sessionId]);

  const stage = async (cmd, withArgs) => {
    setBusy(true);
    // The session this expansion was asked for. `alive` alone is not
    // enough: the picker can stay mounted while `sessionId` changes
    // underneath it, and staging a command expanded against the
    // previous session's cwd into a different thread is worse than
    // staging nothing.
    const forSession = sessionId;
    try {
      const d = await api.expandCommand(forSession, cmd.name, withArgs);
      // Expansion is not cancellable, so the guard is on the result:
      // without it, closing the sheet mid-flight still injected the
      // staged text into the composer afterwards — thousands of words
      // arriving because of a tap the user had already backed out of.
      if (!alive.current || forSession !== currentSession.current) return;
      onStage({ name: d.name, text: d.text, chars: d.text.length, restricts_tools: d.restricts_tools });
      onClose();
    } catch {
      if (!alive.current || forSession !== currentSession.current) return;
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
    <PickerSheet
      label="Insert a command"
      placeholder="Filter commands\u2026"
      items={all}
      failed={failed}
      match={(c, needle) =>
        c.name.toLowerCase().includes(needle) ||
        (c.description || '').toLowerCase().includes(needle)
      }
      empty="No commands in this project."
      noMatch="Nothing matches that."
      onClose={onClose}
    >
      {(c) => {
          const open = chosen?.name === c.name;
          return (
            <div key={c.name} style={PICKER_ROW_STYLE}>
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
        }}
    </PickerSheet>
  );
}
