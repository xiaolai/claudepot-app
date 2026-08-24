// Sign in. Password is the bootstrap and recovery credential; a passkey
// is the day-to-day login once one has been registered from an already
// authenticated session.
import { useEffect, useState } from 'react';

import { ApiError, OfflineError, api } from './api.js';
import { getAssertion, passkeyBlocker, passkeySupport } from './webauthn.js';

const { Btn, Ico, Surface } = window;

/** Turn the server's error slug into something a person can act on. */
function explain(e) {
  if (e instanceof OfflineError) return 'Cannot reach this Mac. Check the connection and try again.';
  if (!(e instanceof ApiError)) return 'Something went wrong.';
  switch (e.code) {
    case 'invalid':
      // Deliberately does not say which half was wrong — the server
      // does not tell us, and guessing would be worse than silence.
      return 'That did not work.';
    case 'totp_required':
      return 'Enter the code from your authenticator app.';
    case 'throttled':
      return `Too many attempts. Try again in ${e.retryAfterSecs ?? 30}s.`;
    case 'not_configured':
      return 'No password is set on this Mac yet. Run `claudepot remote set-password` there first.';
    case 'no_passkey':
      return 'No passkey is registered. Sign in with your password, then add one from Settings.';
    default:
      return 'Something went wrong.';
  }
}

function defaultLabel() {
  const ua = navigator.userAgent || '';
  if (/iPhone/.test(ua)) return 'iPhone';
  if (/iPad/.test(ua)) return 'iPad';
  if (/Android/.test(ua)) return 'Android phone';
  if (/Macintosh/.test(ua)) return 'Mac';
  return 'browser';
}

export function Login({ onSignedIn }) {
  const [password, setPassword] = useState('');
  const [totp, setTotp] = useState('');
  const [needTotp, setNeedTotp] = useState(false);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState(null);
  const [support, setSupport] = useState(null);

  useEffect(() => {
    let cancelled = false;
    passkeySupport().then((s) => !cancelled && setSupport(s));
    return () => {
      cancelled = true;
    };
  }, []);

  const submit = async (e) => {
    e.preventDefault();
    if (busy) return;
    setBusy(true);
    setMsg(null);
    try {
      const res = await api.login(password, totp, defaultLabel());
      setPassword('');
      setTotp('');
      onSignedIn(res.token);
    } catch (err) {
      if (err instanceof ApiError && err.code === 'totp_required') setNeedTotp(true);
      setMsg(explain(err));
    } finally {
      setBusy(false);
    }
  };

  const usePasskey = async () => {
    if (busy) return;
    setBusy(true);
    setMsg(null);
    try {
      const begin = await api.passkeyLoginBegin();
      const assertion = await getAssertion(begin.options);
      const res = await api.passkeyLoginFinish({ ...assertion, challenge_id: begin.challenge_id });
      onSignedIn(res.token);
    } catch (err) {
      if (err?.name === 'NotAllowedError' || err?.message === 'cancelled') setMsg(null);
      else setMsg(explain(err));
    } finally {
      setBusy(false);
    }
  };

  const blocker = support ? passkeyBlocker(support) : null;

  return (
    <div
      className="sc"
      style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', justifyContent: 'center', padding: 'var(--gut)' }}
    >
      <h1 className="disp" style={{ fontSize: 'var(--t-hero)', marginBottom: 'var(--s2)' }}>
        Claudepot
      </h1>
      <p style={{ color: 'var(--fg3)', fontSize: 'var(--t-sub)', marginBottom: 'var(--s7)' }}>
        Sign in to watch and steer the Claude Code sessions on this Mac.
      </p>

      <Surface>
        <form onSubmit={submit}>
          <Field
            id="pw"
            label="Password"
            type="password"
            autoComplete="current-password"
            value={password}
            onChange={setPassword}
            required
          />
          {needTotp && (
            <Field
              id="totp"
              label="Authenticator code"
              inputMode="numeric"
              autoComplete="one-time-code"
              value={totp}
              onChange={setTotp}
            />
          )}
          <Btn kind="primary" full big disabled={busy || !password}>
            {busy ? 'Signing in…' : 'Sign in'}
          </Btn>
        </form>

        {support?.usable && (
          <div style={{ marginTop: 'var(--s4)' }}>
            <Btn kind="quiet" full ico="key" onClick={usePasskey} disabled={busy}>
              Use passkey
            </Btn>
          </div>
        )}

        {msg && (
          <p role="alert" style={{ marginTop: 'var(--s4)', color: 'var(--dg)', fontSize: 'var(--t-meta)' }}>
            {msg}
          </p>
        )}
      </Surface>

      {support && !support.usable && blocker && (
        <p style={{ marginTop: 'var(--s5)', color: 'var(--fg4)', fontSize: 'var(--t-micro)', lineHeight: 'var(--lh-body)' }}>
          <Ico n="info" s="2xs" w="reg" /> {blocker}
        </p>
      )}
    </div>
  );
}

function Field({ id, label, value, onChange, ...rest }) {
  return (
    <label htmlFor={id} style={{ display: 'block', marginBottom: 'var(--s4)' }}>
      <span
        style={{
          display: 'block',
          fontSize: 'var(--t-micro)',
          fontWeight: 'var(--w-semi)',
          color: 'var(--fg3)',
          letterSpacing: 'var(--ls-wide)',
          marginBottom: 'var(--s2)',
        }}
      >
        {label}
      </span>
      <input
        id={id}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        {...rest}
        style={{
          width: '100%',
          height: 'var(--ctl-lg)',
          padding: '0 var(--s4)',
          borderRadius: 'var(--r-md)',
          background: 'var(--sf2)',
          color: 'var(--fg)',
          // 16px is the floor that stops iOS zooming the viewport on
          // focus; the design's --t-body is exactly that.
          fontSize: 'var(--t-body)',
          fontFamily: 'inherit',
        }}
      />
    </label>
  );
}
