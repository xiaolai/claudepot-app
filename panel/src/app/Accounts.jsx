// Accounts — read-only.
//
// Switching is the one verb this screen obviously wants and the one it
// must not have yet. A swap either fails while Claude Code is running or
// bypasses the keychain-drift guard that stops a rotated blob being
// written under the wrong label; either needs its own check before it is
// reachable from a phone.
import { useCallback, useEffect, useState } from 'react';

import { OfflineError, api, newIdempotencyKey } from './api.js';
import { ago, verifyChip } from './format.js';
import { Muted } from './views.jsx';

const { Chip, Face, Group, Item, List, Meter, Surface, Tap } = window;

/**
 * The rate-limit windows, in the order the desktop's `UsageBlock` shows
 * them so the two surfaces read the same way round.
 *
 * Rendered only where the server actually sent a figure. Anthropic
 * returns `five_hour` and `seven_day` on these accounts today and omits
 * the per-model pair; drawing a ring at 0% for a window nobody measured
 * would be a number invented by the client, which is the thing the
 * panel's rules refuse everywhere else.
 */
const WINDOWS = [
  { key: 'five_hour', sub: '5h' },
  { key: 'seven_day', sub: '7d' },
  { key: 'seven_day_opus', sub: 'opus' },
  { key: 'seven_day_sonnet', sub: 'sonnet' },
];

/** Stable hue per account so the tinted face is recognisable. */
function hueOf(email) {
  let h = 0;
  for (let i = 0; i < email.length; i += 1) h = (h * 31 + email.charCodeAt(i)) % 360;
  return h;
}

/**
 * Every window the server reported a figure for: the fixed plan-level
 * ones first, then whatever it scoped to a model.
 *
 * The scoped list is open-ended by design — it carries the weekly Fable
 * limit today and will carry the next model without a code change,
 * which is exactly why it cannot be a fixed key list like `WINDOWS`.
 * Labels are server-supplied and already human-readable, so they are
 * rendered verbatim rather than mapped.
 */
function metersOf(a) {
  const named = WINDOWS.map((win) => ({ ...win, w: a.usage?.[win.key] })).filter(
    (m) => m.w && typeof m.w.utilization === 'number',
  );
  const scoped = (a.usage?.scoped ?? [])
    .filter((sc) => sc && typeof sc.utilization === 'number')
    .map((sc) => ({ key: `scoped:${sc.label}`, sub: sc.label, w: sc }));
  return [...named, ...scoped];
}

/**
 * One account, rendered identically in the active card and in the list.
 *
 * Shared rather than duplicated: the two differ only in their container,
 * and a second copy of this markup is a second place for the meter row
 * or the switch label to drift.
 */
function AccountRow({ a, v, meters, busy, onActivate }) {
  return (
    <>
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
      {/* No plan chip and no Desktop chip. The plan
          is the same on every row here, so it separates
          nothing; the Desktop slot is not something
          this surface can move, and naming it invited
          the reading that the control beside it might.
          What is left is only what differs and matters:
          a verification state, when there is one. */}
      {v && (
        <Chip tone={v.tone} size="xs">
          {v.label}
        </Chip>
      )}
    </div>
    {/* Under the identity rather than beside it: three
        72px rings do not fit in a row that also holds an
        avatar, an email and a control at 390px, and
        shrinking them re-creates the crowding that made
        `sm` wrong in the first place. Wrapping, so a
        fourth window appearing costs a line rather than
        the layout. */}
    {meters.length > 0 && (
      <div
        style={{
          display: 'flex',
          gap: 'var(--s3)',
          flexWrap: 'wrap',
          marginTop: 'var(--s3)',
        }}
      >
        {meters.map((m) => (
          <Meter
            key={m.key}
            pct={Math.round(m.w.utilization)}
            size="md"
            sub={m.sub}
          />
        ))}
      </div>
    )}
    {a.usage_as_of && (
      <div className="mono" style={{ fontSize: 'var(--t-nano)', color: 'var(--fg4)', marginTop: 'var(--s2)' }}>
        usage as of {ago(a.usage_as_of)} ago
      </div>
    )}
  </div>
  {/* Icon-only, which the row earns: it repeats down a
      list, it is never the screen's primary action, and
      `check` reads as "make this the one" across the
      web. The label still names the SLOT — `cli` and
      `desktop` are independent nouns and this moves
      only the first — and it is not the sole disclosure:
      the note under the list says the same in prose,
      because a tooltip is not a place to hide something
      the reader needs. */}
  {!a.is_cli_active && (
    <Tap
      n={busy === a.email ? 'clock' : 'check'}
      onClick={busy ? undefined : () => onActivate(a.email, false)}
      label={`Use ${a.email} for the CLI`}
    />
  )}
    </>
  );
}

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
      setNote(
        r.already_active
          ? `${email} was already the CLI account.`
          : `Claude Code now uses ${email}. Claude Desktop is unchanged.`,
      );
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
  // The CLI account is lifted out of the list entirely — see the card
  // below for why. `find` rather than a sort, because there is exactly
  // one CLI slot and a second match would be a bug worth not hiding.
  const active = accounts?.find((a) => a.is_cli_active) ?? null;
  const others = accounts?.filter((a) => !a.is_cli_active) ?? [];

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

      {active && (
        <Group>
          {/* A card, not a tinted row. "Which account is Claude Code
              using" is the question this screen exists to answer, and a
              row that differs only by a chip makes it something you
              work out rather than something you see. Lifting it out of
              the list also removes the need to mark it: nothing else is
              up here. */}
          <Surface>
            {/* `Item` supplies the flex row in the list; a Surface does
                not, so the card brings its own. Same gap and alignment,
                so the two containers put the row on screen identically. */}
            <div style={{ display: 'flex', gap: 'var(--s3)', alignItems: 'flex-start' }}>
            <AccountRow
              a={active}
              v={verifyChip(active.verify_status)}
              meters={metersOf(active)}
              busy={busy}
              onActivate={activate}
            />
            </div>
          </Surface>
        </Group>
      )}

      {others.length > 0 && (
        <Group title={active ? 'Other accounts' : undefined}>
          <List>
            {others.map((a, i) => (
              <Item key={a.email} first={i === 0} style={{ alignItems: 'flex-start' }}>
                <AccountRow
                  a={a}
                  v={verifyChip(a.verify_status)}
                  meters={metersOf(a)}
                  busy={busy}
                  onActivate={activate}
                />
              </Item>
            ))}
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
        Switching sets which account Claude Code uses. Claude Desktop is a separate slot and is
        not touched from here. Adding, removing and verifying accounts stay at the machine.
        {data?.usage_source === 'none' && ' Usage figures appear once the Claudepot app has run.'}
      </p>
    </div>
  );
}
