// The shell: which screen is showing, and nothing else.
//
// Auth, polling and theme live in `useSession`; the thread's transcript
// lives in `useTranscript`. What is left here is routing — the tab bar,
// the push view, and the decision between them.
//
// ## Three steps, and only the last one restructures
//
// `data-bp` is written from `useBP`'s ResizeObserver on THIS element,
// so one observation drives type, gutter and layout together. It reads
// the panel's own width rather than the display's, which is what makes
// a 390px split view render as a phone on a 27" monitor.
//
// A container query cannot do this. `@container` matches DESCENDANTS of
// the container and never the container element itself — and the panel
// is the element that steps.
//
//   sm  ≤479px    bottom tabs, one column, the thread covers the list
//   md  480–899   bottom tabs, one column, wider gutter and larger type
//   lg  ≥900      left icon rail, list and thread side by side
import { useCallback, useEffect, useRef, useState } from 'react';

import { Login } from './Login.jsx';
import { Sessions } from './Sessions.jsx';
import { Thread } from './Thread.jsx';
import { Accounts } from './Accounts.jsx';
import { Settings } from './Settings.jsx';
import { useAuth, useSessions, useTheme, useToolDisplay } from './useSession.js';
import { useOutboxDrain, useTotalQueued } from './useOutbox.js';

const { Badge, Ico, IcoFill, Wire, useBP } = window;

const TABS = [
  { v: 'sessions', ico: 'layers', n: 'Sessions' },
  { v: 'accounts', ico: 'user', n: 'Accounts' },
  { v: 'settings', ico: 'sliders', n: 'Settings' },
];

function TabBtn({ t, on, badge, rail, onGo }) {
  const n = t.v === 'sessions' ? badge : 0;
  return (
    <button
      onClick={() => onGo(t.v)}
      aria-label={t.n}
      aria-current={on || undefined}
      style={{
        // In the bar each button takes an equal share of the width; in
        // the rail they stack and each takes the full width instead.
        flex: rail ? '0 0 auto' : 1,
        width: rail ? '100%' : 'auto',
        minHeight: 'var(--tab-h)',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 'var(--s1)',
        borderRadius: 'var(--r-md)',
        // The rail has no bar beneath it to say which tab is current,
        // so the active one carries its own ground.
        background: rail && on ? 'var(--ac-wash)' : 'transparent',
        transition: 'background var(--d1) var(--e)',
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

/**
 * The same buttons, stood up on the left. Wide layout only.
 *
 * A rail rather than a wider bar because at ≥900px the bottom edge is a
 * long way from the content and from the hands: on a tablet the thumb
 * rests at the side, and on a desktop window the bottom of the screen is
 * nowhere near the pointer. It also gives the thread its full height,
 * which is the whole reason to have this step.
 */
function TabRail({ view, onGo, badge }) {
  return (
    <nav
      style={{
        flexShrink: 0,
        width: 'var(--rail-w)',
        display: 'flex',
        flexDirection: 'column',
        gap: 'var(--s1)',
        padding: 'var(--s3) var(--s2)',
        background: 'var(--bg-deep)',
        zIndex: 'var(--z-bar)',
        boxShadow: 'inset calc(var(--bw-hair) * -1) 0 0 var(--hair)',
      }}
    >
      {TABS.map((t) => (
        <TabBtn key={t.v} t={t} on={view === t.v} badge={badge} rail onGo={onGo} />
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

  // The step is measured off this element, so the ref has to reach the
  // outermost `.panel` div — see the header comment on why this is not a
  // container query.
  const shell = useRef(null);
  const bp = useBP(shell);
  const wide = bp === 'lg';

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

  // Messages typed while this Mac was unreachable, sent as soon as it
  // answers. At the shell rather than in the thread: "sends when the Mac
  // is back" has to hold whether or not the user is still looking at the
  // conversation they typed into.
  useOutboxDrain(sessions, conn, refresh);
  const held = useTotalQueued();

  useEffect(() => {
    setHost(window.location.hostname);
  }, []);

  // Every branch returns a `Shell`, and that is load-bearing rather
  // than tidy. `useBP` observes `shell.current` in an effect keyed on
  // the ref object, which is stable — so the effect runs exactly once,
  // after the first commit. A boot screen that rendered OUTSIDE the
  // shell left `shell.current` null at that moment and the observer was
  // never attached: `data-bp` stayed `sm` on every screen forever, which
  // is the same dead-CSS failure this change exists to fix.
  if (!checked) {
    return (
      <Shell theme={theme} bp={bp} innerRef={shell}>
        <Boot />
      </Shell>
    );
  }

  if (!token) {
    return (
      <Shell theme={theme} bp={bp} innerRef={shell}>
        <Column measure="measure-form">
          <Login onSignedIn={signIn} />
        </Column>
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
        openId={wide ? openId : null}
        onOpen={setOpenId}
        onRetry={refresh}
        onChanged={refresh}
      />
    );

  const thread = open && (
    <Thread
      session={open}
      onBack={() => setOpenId(null)}
      onChanged={refresh}
      conn={conn}
      tools={tools}
      inPane={wide}
    />
  );

  // Narrow: one column, and the thread covers the list.
  // Wide: the list keeps its place and the thread opens beside it. The
  // other two tabs have no detail view, so they take the full width
  // rather than leaving an empty pane next to them.
  const stage = wide ? (
    <div style={{ flex: 1, minWidth: 0, display: 'flex' }}>
      <div
        // Accounts and Settings have no detail pane, so they take the
        // whole width — and then need a reading measure, or a list row
        // in a 1600px window is 1600px of hairline. See `controls.css`.
        className={view === 'sessions' ? undefined : 'measure'}
        style={{
          flexShrink: 0,
          width: view === 'sessions' ? 'var(--pane-list)' : 'auto',
          flex: view === 'sessions' ? undefined : 1,
          minWidth: 0,
          display: 'flex',
          flexDirection: 'column',
          boxShadow:
            view === 'sessions' ? 'inset calc(var(--bw-hair) * -1) 0 0 var(--hair)' : 'none',
        }}
      >
        {screen}
      </div>
      {view === 'sessions' && (
        <div
          style={{
            flex: 1,
            minWidth: 0,
            display: 'flex',
            flexDirection: 'column',
            background: 'var(--bg-deep)',
          }}
        >
          {thread || (
            <div style={{ flex: 1, display: 'grid', placeItems: 'center', padding: 'var(--gut)' }}>
              <p
                className="disp"
                style={{ fontSize: 'var(--t-title)', color: 'var(--fg4)', textAlign: 'center' }}
              >
                Pick a session
              </p>
            </div>
          )}
        </div>
      )}
    </div>
  ) : (
    thread || screen
  );

  return (
    <Shell theme={theme} bp={bp} innerRef={shell}>
      {wide && (
        <TabRail
          view={view}
          onGo={(v) => {
            setView(v);
          }}
          badge={attention}
        />
      )}
      <Column>
      {/* The only way out of a stale bundle on a home-screen app.
          Installed to the iOS home screen there is no address bar, no
          reload button, and the system pull-to-refresh cannot fire
          because the shell is `position: fixed` and the document never
          scrolls — so without this the app runs last week's build
          indefinitely with nothing saying so.

          Above the thread as well as the list: a stale bundle is stale
          wherever you happen to be standing. */}
      {stale && <Updated />}
      {/* Suppressed under a thread on a narrow screen only: there the
          transcript covers the list and the composer says the same
          thing at the point of use. In the wide layout the list is
          still on screen beside it, so the banner belongs to the list
          and stays. */}
      {conn !== 'online' && (wide || !open) && (
        <Wire state={conn} queued={held} onRetry={refresh} host={host || 'this Mac'} />
      )}
      {stage}
      {/* The tab bar persists into a thread. A conversation is a place
          in the app, not a mode you have to back out of: losing the
          bar meant the only way to reach Accounts from a transcript
          was Back first, and nothing on screen said so. Tapping a tab
          therefore also leaves the thread — otherwise the tab would
          light up under a transcript that is still covering it.

          Wide is the exception, and for the same reason inverted: the
          thread is beside the list rather than over it, so switching
          tabs hides nothing and closing the conversation would be an
          unasked-for navigation. `TabRail` above does not clear it. */}
      {!wide && (
        <TabBar
          view={view}
          onGo={(v) => {
            setOpenId(null);
            setView(v);
          }}
          badge={attention}
        />
      )}
      </Column>
    </Shell>
  );
}

/**
 * The measured element.
 *
 * `data-bp` used to be pinned to `"sm"`, which left the `md` and `lg`
 * token steps in `ds-tokens.css` as dead CSS — the panel rendered at the
 * phone floor on every screen it was ever opened on. The ref is what
 * turns them back on; nothing else about this element changed.
 *
 * A row, not a column: the wide layout's rail is a sibling of the
 * content column rather than something inside it. With no rail (every
 * narrow layout, and the sign-in screen) the single `Column` child fills
 * the row and the direction is invisible.
 */
function Shell({ theme, bp, innerRef, children }) {
  return (
    <div
      ref={innerRef}
      data-t={theme === 'system' ? undefined : theme}
      data-bp={bp}
      className="panel"
      style={{
        position: 'fixed',
        inset: 0,
        display: 'flex',
        background: 'var(--bg)',
        color: 'var(--fg)',
        overflow: 'hidden',
      }}
    >
      {children}
    </div>
  );
}

/** Everything that is not the rail: header, stage, tab bar. */
function Column({ measure, children }) {
  return (
    <div
      className={measure}
      style={{
        flex: 1,
        minWidth: 0,
        display: 'flex',
        flexDirection: 'column',
        overflow: 'hidden',
      }}
    >
      {children}
    </div>
  );
}

/** Inside the shell, which already paints the background and is fixed. */
function Boot() {
  return (
    <div style={{ flex: 1, display: 'grid', placeItems: 'center' }}>
      <span style={{ color: 'var(--fg4)', fontSize: 'var(--t-meta)' }}>…</span>
    </div>
  );
}
