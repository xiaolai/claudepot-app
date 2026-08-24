// Settings.
//
// The delivered design carries notification toggles. They are not here:
// push delivery has no transport on this surface, and a switch that
// stores a preference nothing reads is worse than an absent feature —
// it tells the user they will be notified.
import { useCallback, useEffect, useRef, useState } from 'react';

import { ApiError, OfflineError, api, newIdempotencyKey } from './api.js';
import { until } from './format.js';
import { createCredential, passkeyBlocker, passkeySupport } from './webauthn.js';

const { Btn, Chip, Group, Ico, Item, List, Seg } = window;

/**
 * "in 12d", "expired", "never", or "—" while we do not know yet.
 *
 * The unknown case is the point. `me` is `null` before `/api/me`
 * answers and after it fails, and collapsing that into `'never'` stated
 * — as a fact, on the security pane — that this session token does not
 * expire. A dash says nothing, which is the true thing to say.
 */
function expiry(me) {
  if (!me) return '—';
  if (!me.expires_at) return 'never';
  const left = until(me.expires_at);
  return left ? `in ${left}` : 'expired';
}

const THEMES = [
  { v: 'system', n: 'System' },
  { v: 'light', n: 'Light' },
  { v: 'dark', n: 'Dark' },
];

const TOOLS = [
  { v: 'grouped', n: 'Grouped' },
  { v: 'shown', n: 'Every one' },
];

export function Settings({ theme, onTheme, tools, onTools, host, onSignOut, onRefresh }) {
  const [me, setMe] = useState(null);
  const [inbound, setInbound] = useState(null);
  const [support, setSupport] = useState(null);
  const [passkeys, setPasskeys] = useState(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState(null);

  // Set to false on unmount. Every setter behind an await checks it —
  // signing out while these are in flight would otherwise set state on a
  // component that is gone, and React logs that as an error the render
  // check would then fail on.
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const reload = useCallback(async () => {
    try {
      const m = await api.me();
      if (!alive.current) return;
      setMe(m);
      setPasskeys(m.passkeys ?? 0);
    } catch {
      /* The shell owns the auth lifecycle. */
    }
    try {
      const st = await api.inbound();
      if (alive.current) setInbound(st);
    } catch {
      if (alive.current) setInbound(null);
    }
  }, []);

  useEffect(() => {
    reload();
    passkeySupport().then((s) => {
      if (alive.current) setSupport(s);
    });
  }, [reload]);

  const addPasskey = async () => {
    if (busy) return;
    setBusy(true);
    setNotice(null);
    try {
      const begin = await api.passkeyRegisterBegin();
      const attestation = await createCredential(begin.options);
      await api.passkeyRegisterFinish(
        { ...attestation, challenge_id: begin.challenge_id },
        newIdempotencyKey(),
      );
      // The component has an `alive` ref precisely because these awaits
      // are long — the WebAuthn prompt is a system dialog the user can
      // sit on — and this path was the one that did not consult it.
      if (!alive.current) return;
      setNotice('Passkey added. Next sign-in can use Face ID.');
      reload();
    } catch (e) {
      if (!alive.current) return;
      if (e?.name === 'NotAllowedError' || e?.message === 'cancelled') setNotice(null);
      else if (e instanceof OfflineError) setNotice('Cannot reach this Mac.');
      else if (e instanceof ApiError && e.code === 'rp_id_unavailable')
        setNotice('This origin has no hostname — reach the panel by name, not by IP address.');
      else setNotice('Could not add a passkey.');
    } finally {
      if (alive.current) setBusy(false);
    }
  };

  const closeInbound = async () => {
    if (busy) return;
    setBusy(true);
    try {
      await api.inboundRevoke(newIdempotencyKey());
      setNotice('Remote-message window closed.');
      reload();
      onRefresh?.();
    } catch {
      setNotice('Could not close it.');
    } finally {
      setBusy(false);
    }
  };

  const blocker = support ? passkeyBlocker(support) : null;

  return (
    <div className="sc" style={{ flex: 1, minHeight: 0, padding: '0 var(--gut) var(--s8)' }}>
      <header style={{ padding: 'var(--s6) 0 var(--s2)' }}>
        <h1 className="disp" style={{ fontSize: 'var(--t-hero)' }}>
          Settings
        </h1>
      </header>

      <Group title="Appearance">
        <List>
          <Item first>
            <span style={{ flex: 1, fontSize: 'var(--t-sub)' }}>Theme</span>
            <Seg opts={THEMES} v={theme} onChange={onTheme} />
          </Item>
          <Item>
            <span style={{ flex: 1, fontSize: 'var(--t-sub)' }}>
              Tool calls
              {/* The reason for the default, where the person changing
                  it can read it: measured across five real sessions on
                  this machine, 59–91% of transcript rows were tool
                  ticks. */}
              <span
                style={{
                  display: 'block',
                  fontSize: 'var(--t-nano)',
                  color: 'var(--fg4)',
                  marginTop: 'var(--s1)',
                }}
              >
                Grouped folds a run into one row you can open.
              </span>
            </span>
            <Seg opts={TOOLS} v={tools} onChange={onTools} />
          </Item>
        </List>
      </Group>

      <Group title="Sign-in">
        <List>
          <Item first>
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 'var(--t-sub)' }}>Passkey</div>
              <div style={{ fontSize: 'var(--t-micro)', color: 'var(--fg4)', marginTop: 'var(--s1)' }}>
                {/* `null` is "not loaded", `0` is "none". Conflating
                    them announced "None registered" before /api/me
                    answered — and after it failed — which is the one
                    claim that would send someone to enrol a passkey
                    they already have. */}
                {passkeys === null
                  ? 'Checking…'
                  : passkeys > 0
                    ? `${passkeys} registered`
                    : 'None registered'}
              </div>
            </div>
            {support?.usable ? (
              <Btn kind="quiet" ico="key" onClick={addPasskey} disabled={busy}>
                Add
              </Btn>
            ) : (
              <Chip tone="quiet" size="xs">
                Unavailable
              </Chip>
            )}
          </Item>
        </List>
        {support && !support.usable && blocker && (
          <p style={{ marginTop: 'var(--s2)', fontSize: 'var(--t-micro)', color: 'var(--fg4)', lineHeight: 'var(--lh-body)' }}>
            {blocker}
          </p>
        )}
        <p style={{ marginTop: 'var(--s2)', fontSize: 'var(--t-micro)', color: 'var(--fg4)', lineHeight: 'var(--lh-body)' }}>
          The password stays the way in if a passkey is lost. Adding one needs a session that is already
          signed in — which is this one.
        </p>
      </Group>

      <Group title="Remote messages">
        <List>
          <Item first>
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 'var(--t-sub)' }}>Delivery window</div>
              <div style={{ fontSize: 'var(--t-micro)', color: 'var(--fg4)', marginTop: 'var(--s1)' }}>
                {/* `unmanaged_open` FIRST. Core's `is_unmanaged_open()`
                    is `is_open() && nothing-is-minding-it`, so it
                    implies `open` — testing `open` first made this
                    branch unreachable and the warning it carries has
                    never once been shown. Its own doc comment says
                    "worth saying out loud in any UI", and "open with a
                    deadline" and "open forever" are not the same risk;
                    the desktop chip already draws that line. */}
                {inbound === null
                  ? '—'
                  : inbound.unmanaged_open
                    ? 'Open, but set outside Claudepot — nothing will close it'
                    : inbound.open
                      ? 'Open — sessions accept messages without holding them'
                      : 'Closed — messages are held for approval at the machine'}
              </div>
            </div>
            {inbound?.open && (
              <Btn kind="danger" onClick={closeInbound} disabled={busy}>
                Close
              </Btn>
            )}
          </Item>
        </List>
        <p style={{ marginTop: 'var(--s2)', fontSize: 'var(--t-micro)', color: 'var(--fg4)', lineHeight: 'var(--lh-body)' }}>
          It can only be closed from here, never opened. Opening it would let one stolen token both open
          the door and walk through it.
        </p>
      </Group>

      <Group title="This device">
        <List>
          <Item first>
            <span style={{ flex: 1, fontSize: 'var(--t-sub)' }}>Name</span>
            <span className="mono" style={{ fontSize: 'var(--t-meta)', color: 'var(--fg3)' }}>
              {me?.device ?? '—'}
            </span>
          </Item>
          <Item>
            <span style={{ flex: 1, fontSize: 'var(--t-sub)' }}>Session expires</span>
            <span className="mono" style={{ fontSize: 'var(--t-meta)', color: 'var(--fg3)' }}>
              {expiry(me)}
            </span>
          </Item>
          <Item>
            <span style={{ flex: 1, fontSize: 'var(--t-sub)' }}>Host</span>
            <span className="mono" style={{ fontSize: 'var(--t-meta)', color: 'var(--fg3)' }} title={host}>
              {host || '—'}
            </span>
          </Item>
          {/* Which Claudepot is serving this panel. The bundle is
              embedded in the server binary, so a `remote serve` that
              outlives a rebuild serves the old panel indefinitely and
              nothing else on this screen would say so — a fixed bug
              reads as unfixed, from the phone, with no way to tell. */}
          <Item>
            <span style={{ flex: 1, fontSize: 'var(--t-sub)' }}>Server</span>
            <span className="mono" style={{ fontSize: 'var(--t-meta)', color: 'var(--fg3)' }}>
              {me?.server_version ? `v${me.server_version}` : '—'}
            </span>
          </Item>
          {/* The manual half of reloading. Installed to the iOS home
              screen this app has no address bar, no reload button, and
              cannot use the system pull-to-refresh — the shell is
              `position: fixed` so the document never scrolls. Without a
              control here there is no way to reload at all short of
              deleting the icon and re-adding it.

              The bar that appears when the server version changes
              handles the case you would notice; this handles the rest. */}
          <Item>
            <span style={{ flex: 1, fontSize: 'var(--t-sub)' }}>
              Reload
              <span
                style={{
                  display: 'block',
                  fontSize: 'var(--t-nano)',
                  color: 'var(--fg4)',
                  marginTop: 'var(--s1)',
                }}
              >
                Fetches the current app. Nothing is signed out.
              </span>
            </span>
            <Chip tone="quiet" size="xs" onClick={() => window.location.reload()}>
              Reload
            </Chip>
          </Item>
        </List>
      </Group>

      {notice && (
        <p role="status" style={{ marginTop: 'var(--s5)', fontSize: 'var(--t-meta)', color: 'var(--fg3)' }}>
          {notice}
        </p>
      )}

      <div style={{ marginTop: 'var(--s7)' }}>
        <Btn kind="danger" full onClick={onSignOut}>
          Sign out
        </Btn>
      </div>

      <p style={{ marginTop: 'var(--s5)', fontSize: 'var(--t-micro)', color: 'var(--fg4)', lineHeight: 'var(--lh-body)' }}>
        <Ico n="info" s="2xs" w="reg" /> Whoever holds this session can drive Claude Code on this Mac.
        Revoke a lost device with <span className="mono">claudepot remote revoke-all</span> at the machine.
      </p>
    </div>
  );
}
