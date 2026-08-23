// Accounts — read-only.
//
// Switching is the one verb this screen obviously wants and the one it
// must not have yet. A swap either fails while Claude Code is running or
// bypasses the keychain-drift guard that stops a rotated blob being
// written under the wrong label; either needs its own check before it is
// reachable from a phone.
import { useCallback, useEffect, useState } from 'react';

import { OfflineError, api, newIdempotencyKey } from './api.js';
import { ago } from './format.js';
import { Muted } from './views.jsx';

const { Chip, Face, Group, Item, List, Meter } = window;

/** Stable hue per account so the tinted face is recognisable. */
function hueOf(email) {
  let h = 0;
  for (let i = 0; i < email.length; i += 1) h = (h * 31 + email.charCodeAt(i)) % 360;
  return h;
}

const VERIFY_TONE = {
  ok: null,
  drift: { tone: 'warn', label: 'Drift' },
  rejected: { tone: 'danger', label: 'Rejected' },
  signed_out: { tone: 'danger', label: 'Signed out' },
  network_error: { tone: 'quiet', label: 'Unverified' },
  never: { tone: 'quiet', label: 'Never verified' },
};

export function Accounts() {
  const [data, setData] = useState(null);
  const [error, setError] = useState(null);
  const [busy, setBusy] = useState(null);
  // Which account hit the live-session gate, so the override appears on
  // that row and nowhere else.
  const [conflict, setConflict] = useState(null);
  const [note, setNote] = useState(null);

  const load = useCallback((signal) => {
    api
      .accounts(signal)
      .then(setData)
      .catch((e) => {
        if (e?.name === 'AbortError') return;
        setError(e instanceof OfflineError ? 'offline' : 'error');
      });
  }, []);

  useEffect(() => {
    const ctrl = new AbortController();
    load(ctrl.signal);
    return () => ctrl.abort();
  }, [load]);

  const activate = async (email, force) => {
    setBusy(email);
    setNote(null);
    if (!force) setConflict(null);
    try {
      const r = await api.activateAccount(email, force, newIdempotencyKey());
      setNote(r.already_active ? `${email} was already active.` : `Claude Code now uses ${email}.`);
      setConflict(null);
      load();
    } catch (e) {
      if (e?.status === 409) {
        // Not an error to apologise for — it is the gate doing its job,
        // and the next move belongs to the user.
        setConflict(email);
      } else {
        setNote(e instanceof OfflineError ? 'Cannot reach this Mac.' : e?.message || 'Switch failed.');
      }
    } finally {
      setBusy(null);
    }
  };

  const accounts = data?.accounts ?? null;

  return (
    <div className="sc" style={{ flex: 1, minHeight: 0, padding: '0 var(--gut) var(--s8)' }}>
      <header style={{ padding: 'var(--s6) 0 var(--s2)' }}>
        <h1 className="disp" style={{ fontSize: 'var(--t-hero)' }}>
          Accounts
        </h1>
      </header>

      {error === 'offline' && <Muted>Cannot reach this Mac.</Muted>}
      {error && error !== 'offline' && <Muted>Could not read the account list.</Muted>}
      {!error && accounts === null && <Muted>Loading…</Muted>}
      {accounts?.length === 0 && <Muted>No accounts registered with Claudepot.</Muted>}

      {accounts?.length > 0 && (
        <Group>
          <List>
            {accounts.map((a, i) => {
              const v = VERIFY_TONE[a.verify_status] ?? VERIFY_TONE.never;
              const five = a.usage?.five_hour;
              return (
                <Item key={a.email} first={i === 0} style={{ alignItems: 'flex-start' }}>
                  <Face name={a.email} hue={hueOf(a.email)} size="md" ring={a.is_cli_active} />
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div
                      style={{
                        fontSize: 'var(--t-sub)',
                        fontWeight: 'var(--w-med)',
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                        whiteSpace: 'nowrap',
                      }}
                    >
                      {a.email}
                    </div>
                    <div style={{ display: 'flex', gap: 'var(--s2)', flexWrap: 'wrap', marginTop: 'var(--s2)' }}>
                      {a.is_cli_active && <Chip tone="accent" size="xs">CLI</Chip>}
                      {a.is_desktop_active && <Chip tone="accent" size="xs">Desktop</Chip>}
                      {a.plan && (
                        <Chip tone="quiet" size="xs">
                          {a.plan}
                        </Chip>
                      )}
                      {v && (
                        <Chip tone={v.tone} size="xs">
                          {v.label}
                        </Chip>
                      )}
                    </div>
                    {a.usage_as_of && (
                      <div className="mono" style={{ fontSize: 'var(--t-nano)', color: 'var(--fg4)', marginTop: 'var(--s2)' }}>
                        usage as of {ago(a.usage_as_of)} ago
                      </div>
                    )}
                  </div>
                  {five && <Meter pct={Math.round(five.utilization)} size="sm" sub="5h" />}
                  {!a.is_cli_active && (
                    <Chip
                      tone="quiet"
                      size="xs"
                      onClick={busy ? undefined : () => activate(a.email, false)}
                    >
                      {busy === a.email ? '…' : 'Use'}
                    </Chip>
                  )}
                </Item>
              );
            })}
          </List>
        </Group>
      )}

      {/* The gate, explained where the decision is made. Inline rather
          than a tooltip: per the design rules a blocked action states
          its reason next to itself, and this one carries a consequence
          the user needs before overriding it. */}
      {conflict && (
        <div
          role="alert"
          style={{
            marginTop: 'var(--s4)',
            padding: 'var(--s3)',
            borderRadius: 'var(--r-md)',
            background: 'var(--wn-wash)',
          }}
        >
          <p style={{ fontSize: 'var(--t-micro)', color: 'var(--fg2)', lineHeight: 'var(--lh-body)' }}>
            Claude Code is running. Switching now works until a running session refreshes its token,
            which puts the old account back — and you would not see it happen from here.
          </p>
          <div style={{ display: 'flex', gap: 'var(--s2)', marginTop: 'var(--s3)' }}>
            <Chip tone="accent" size="md" onClick={busy ? undefined : () => activate(conflict, true)}>
              {busy === conflict ? '…' : 'Switch anyway'}
            </Chip>
            <Chip tone="quiet" size="md" onClick={() => setConflict(null)}>
              Leave it
            </Chip>
          </div>
        </div>
      )}

      {note && (
        <p
          role="status"
          style={{ marginTop: 'var(--s4)', fontSize: 'var(--t-micro)', color: 'var(--fg3)' }}
        >
          {note}
        </p>
      )}

      <p style={{ marginTop: 'var(--s6)', fontSize: 'var(--t-micro)', color: 'var(--fg4)', lineHeight: 'var(--lh-body)' }}>
        Switching sets which account Claude Code uses. Adding, removing and verifying accounts is
        still done at the machine.
        {data?.usage_source === 'none' && ' Usage figures appear once the Claudepot app has run.'}
      </p>
    </div>
  );
}
