// The shell: which screen is showing, and nothing else.
//
// Auth, polling and theme live in `useSession`; the thread's transcript
// lives in `useTranscript`. What is left here is routing — the tab bar,
// the push view, and the decision between them.
//
// The design's wide/tablet layout is deliberately not carried over: it
// was built but only lightly exercised, and it is not on the path to a
// phone client. One column, bottom tabs, thread pushes over the list.
import { useCallback, useEffect, useState } from 'react';

import { Login } from './Login.jsx';
import { Sessions } from './Sessions.jsx';
import { Thread } from './Thread.jsx';
import { Accounts } from './Accounts.jsx';
import { Settings } from './Settings.jsx';
import { useAuth, useSessions, useTheme, useToolDisplay } from './useSession.js';

const { Badge, Ico, IcoFill, Wire } = window;

const TABS = [
  { v: 'sessions', ico: 'layers', n: 'Sessions' },
  { v: 'accounts', ico: 'user', n: 'Accounts' },
  { v: 'settings', ico: 'sliders', n: 'Settings' },
];

function TabBtn({ t, on, badge, onGo }) {
  const n = t.v === 'sessions' ? badge : 0;
  return (
    <button
      onClick={() => onGo(t.v)}
      aria-label={t.n}
      aria-current={on || undefined}
      style={{
        flex: 1,
        minHeight: 'var(--tab-h)',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 'var(--s1)',
        borderRadius: 'var(--r-md)',
      }}
    >
      <span style={{ position: 'relative', display: 'grid', placeItems: 'center' }}>
        {on ? (
          <IcoFill n={t.ico} s="xl" c="var(--ac)" />
        ) : (
          <Ico n={t.ico} s="xl" w="thin" c="var(--fg3)" />
        )}
        {n > 0 && (
          <span
            style={{ position: 'absolute', top: 'calc(var(--s1) * -1)', right: 'calc(var(--s2) * -1)' }}
          >
            <Badge n={n} />
          </span>
        )}
      </span>
      <span
        style={{
          fontSize: 'var(--t-nano)',
          fontWeight: on ? 'var(--w-bold)' : 'var(--w-med)',
          letterSpacing: 'var(--ls-wide)',
          color: on ? 'var(--ac-ink)' : 'var(--fg3)',
        }}
      >
        {t.n}
      </span>
    </button>
  );
}

/**
 * Claudepot restarted with a different version than this bundle came
 * from — so this bundle is old.
 *
 * A tap, not an automatic reload: a reload mid-sentence would lose
 * whatever is in the composer, and "the app updated itself while I was
 * typing" is a worse surprise than a one-line bar.
 *
 * `location.reload()` is enough — the panel is served `no-store`, so a
 * load always fetches the current bytes and there is no cache to bust.
 */
function Updated() {
  return (
    <button
      onClick={() => window.location.reload()}
      style={{
        flexShrink: 0,
        width: '100%',
        padding: 'var(--s2) var(--gut)',
        background: 'var(--ac-wash)',
        color: 'var(--ac-ink)',
        fontSize: 'var(--t-micro)',
        fontWeight: 'var(--w-semi)',
        textAlign: 'center',
      }}
    >
      Claudepot was updated — tap to reload
    </button>
  );
}

function TabBar({ view, onGo, badge }) {
  return (
    <nav
      style={{
        flexShrink: 0,
        display: 'flex',
        background: 'var(--glass)',
        zIndex: 'var(--z-bar)',
        backdropFilter: 'blur(var(--blur-bar))',
        WebkitBackdropFilter: 'blur(var(--blur-bar))',
        boxShadow: 'inset 0 var(--bw-hair) 0 var(--hair)',
        padding: 'var(--s1) var(--s1) var(--safe-b)',
      }}
    >
      {TABS.map((t) => (
        <TabBtn key={t.v} t={t} on={view === t.v} badge={badge} onGo={onGo} />
      ))}
    </nav>
  );
}

export function Panel() {
  const [view, setView] = useState('sessions');
  const [openId, setOpenId] = useState(null);
  const [theme, setTheme] = useTheme();
  const [tools, setTools] = useToolDisplay();
  const [host, setHost] = useState('');

  const { token, checked, signIn, signOut } = useAuth();

  // Routing state is reset here rather than inside `useAuth`, because
  // "which screen was open" is this component's business and the hook
  // has no opinion about it.
  const onSignOut = useCallback(() => {
    signOut();
    setOpenId(null);
    setView('sessions');
  }, [signOut]);

  const { sessions, approvals, conn, refresh, stale } = useSessions({
    enabled: Boolean(token) && checked,
    onUnauthorized: onSignOut,
  });

  useEffect(() => {
    setHost(window.location.hostname);
  }, []);

  if (!checked) return <Boot />;

  if (!token) {
    return (
      <Shell theme={theme}>
        <Login onSignedIn={signIn} />
      </Shell>
    );
  }

  const open = openId && sessions ? sessions.find((s) => s.session_id === openId) : null;
  const attention = (sessions || []).filter((s) => s.live).length;

  const screen = view === 'accounts' ? (
      <Accounts />
    ) : view === 'settings' ? (
      <Settings
        theme={theme}
        onTheme={setTheme}
        tools={tools}
        onTools={setTools}
        host={host}
        onSignOut={onSignOut}
        onRefresh={refresh}
      />
    ) : (
      <Sessions
        sessions={sessions}
        approvals={approvals}
        conn={conn}
        host={host}
        onOpen={setOpenId}
        onRetry={refresh}
        onChanged={refresh}
      />
    );

  return (
    <Shell theme={theme}>
      {/* The only way out of a stale bundle on a home-screen app.
          Installed to the iOS home screen there is no address bar, no
          reload button, and the system pull-to-refresh cannot fire
          because the shell is `position: fixed` and the document never
          scrolls — so without this the app runs last week's build
          indefinitely with nothing saying so.

          Above the thread as well as the list: a stale bundle is stale
          wherever you happen to be standing. */}
      {stale && <Updated />}
      {conn !== 'online' && !open && <Wire state={conn} onRetry={refresh} host={host || 'this Mac'} />}
      {open ? (
        <Thread
          session={open}
          onBack={() => setOpenId(null)}
          onChanged={refresh}
          conn={conn}
          tools={tools}
        />
      ) : (
        screen
      )}
      {/* The tab bar persists into a thread. A conversation is a place
          in the app, not a mode you have to back out of: losing the
          bar meant the only way to reach Projects from a transcript
          was Back first, and nothing on screen said so. Tapping a tab
          therefore also leaves the thread — otherwise the tab would
          light up under a transcript that is still covering it. */}
      <TabBar
        view={view}
        onGo={(v) => {
          setOpenId(null);
          setView(v);
        }}
        badge={attention}
      />
    </Shell>
  );
}

function Shell({ theme, children }) {
  return (
    <div
      data-t={theme === 'system' ? undefined : theme}
      data-bp="sm"
      className="panel"
      style={{
        position: 'fixed',
        inset: 0,
        display: 'flex',
        flexDirection: 'column',
        background: 'var(--bg)',
        color: 'var(--fg)',
        overflow: 'hidden',
      }}
    >
      {children}
    </div>
  );
}

function Boot() {
  return (
    <div
      style={{ position: 'fixed', inset: 0, display: 'grid', placeItems: 'center', background: 'var(--bg)' }}
    >
      <span style={{ color: 'var(--fg4)', fontSize: 'var(--t-meta)' }}>…</span>
    </div>
  );
}
