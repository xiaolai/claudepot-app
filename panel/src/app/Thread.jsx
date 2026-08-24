// The thread. No bubbles: one spine, prose hung off it, tool calls as
// quiet ticks you can expand.
//
// This screen shows raw transcript text. The host redacts a known list
// of secret families and its own module documentation says that list is
// incomplete — GitHub PATs and AWS keys pass through it today. The
// banner says so rather than implying the transcript is scrubbed,
// because a user who believes it is scrubbed will screenshot it.
//
// Loading and paging live in `useTranscript`; following the tail, the
// read mark and sending live in `useThreadState`. What is left here is
// rendering, which is the only thing a reviewer should have to read to
// know what this screen looks like.
import { useEffect, useState } from 'react';

import { api } from './api.js';
import { ago, modelLabel, tightPath, groupTools } from './format.js';
import { Markdown } from './Markdown.jsx';
import { useTranscript } from './useTranscript.js';
import { CommandPicker } from './CommandPicker.jsx';
import { QuickPicker } from './QuickPicker.jsx';
import { useFollowTail, useSendPrompt } from './useThreadState.js';
import { useQueued } from './useOutbox.js';
import { cancel as cancelQueued, retry as retryQueued } from './outbox.js';
import { CopyPath, Muted } from './views.jsx';

const { Btn, Chip, Dot, Ico, Tap } = window;

/**
 * The quick-prompt chips, fetched once per thread.
 *
 * They used to be four hardcoded strings, which is the right list for
 * nobody in particular — the useful ones are the phrases a given person
 * types twenty times a week. They are authored in the desktop app now
 * and read here; see `claudepot-core::quick_prompt`.
 *
 * Failure is silence: a server too old to know the route, or a
 * read that fails, leaves the row empty rather than blocking the
 * composer behind an error nobody can act on from a phone.
 */
function useQuickPrompts() {
  const [prompts, setPrompts] = useState([]);
  useEffect(() => {
    const ctrl = new AbortController();
    api
      .quickPrompts(ctrl.signal)
      .then((d) => setPrompts(d?.prompts ?? []))
      .catch(() => {});
    return () => ctrl.abort();
  }, []);
  return prompts;
}

export function Thread({ session, onBack, onChanged, conn, tools = 'grouped', inPane = false }) {
  const [sheet, setSheet] = useState(null); // 'commands' | 'quick' | null
  const quickPrompts = useQuickPrompts();
  const id = session.session_id;
  const { events, total, loading, error, hasEarlier, loadEarlier } = useTranscript(id);
  const { scroller, onScroll } = useFollowTail(id, total, events);
  const queued = useQueued(id);
  const {
    text,
    setText,
    sending,
    notice,
    send,
    canSend,
    holding,
    blocked,
    warning,
    staged,
    setStaged,
    hasContent,
  } = useSendPrompt(session, conn, onChanged);

  return (
    <>
      <header
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 'var(--s2)',
          height: 'var(--bar-h-back)',
          flexShrink: 0,
          padding: inPane ? '0 var(--s3) 0 var(--gut)' : '0 var(--s3) 0 var(--chev-inset)',
        }}
      >
        {/* In the wide layout the thread sits BESIDE the list rather
            than over it, so there is nothing to go back from — the list
            never left. A chevron there would undo a navigation the user
            did not make. */}
        {!inPane && <Tap n="chevL" onClick={onBack} label="Back" />}
        <div style={{ flex: 1, minWidth: 0 }}>
          <div
            style={{
              fontSize: 'var(--t-sub)',
              fontWeight: 'var(--w-semi)',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
          >
            {session.title || 'Untitled session'}
          </div>
          {/* The canonical copy site for this session's project path.
              Session rows elsewhere are truncated and point here — see
              rules/path-display.md, State B. */}
          <div
            className="mono selectable"
            style={{
              fontSize: 'var(--t-micro)',
              color: 'var(--fg4)',
              overflow: 'hidden',
              whiteSpace: 'nowrap',
            }}
            title={session.project_path || undefined}
          >
            {tightPath(session.project_path, 28)}
            {session.branch ? ` · ${session.branch}` : ''}
            {session.models?.length === 1 ? ` · ${modelLabel(session.models[0])}` : ''}
            {session.models?.length > 1 ? ` · ${session.models.length} models` : ''}
          </div>
        </div>
        <CopyPath path={session.project_path} label="Copy the project path" />
        {session.live && (
          <Dot
            c={session.status === 'waiting' ? 'var(--wn)' : 'var(--ac)'}
            pulse={session.status === 'busy'}
            size="md"
          />
        )}
      </header>

      <RawBanner />

      <div
        ref={scroller}
        onScroll={onScroll}
        className="sc"
        style={{ flex: 1, minHeight: 0, padding: '0 var(--gut)' }}
      >
        {hasEarlier && (
          <div style={{ padding: 'var(--s3) 0', textAlign: 'center' }}>
            <Chip tone="quiet" onClick={loadEarlier}>
              Load earlier
            </Chip>
          </div>
        )}

        {loading && <Muted>Loading…</Muted>}
        {error === 'offline' && <Muted>Cannot reach this Mac.</Muted>}
        {error && error !== 'offline' && <Muted>This transcript could not be read.</Muted>}
        {!loading && !error && events.length === 0 && <Muted>Nothing in this transcript yet.</Muted>}

        <div style={{ position: 'relative', paddingLeft: 'var(--s5)' }}>
          <div
            aria-hidden
            style={{
              position: 'absolute',
              left: 'var(--s2)',
              top: 0,
              bottom: 0,
              width: 'var(--bw-hair)',
              background: 'var(--hair)',
            }}
          />
          {groupTools(events, tools).map((r) =>
            r.kind === 'group' ? (
              <ToolGroup key={`g${r.events[0].index}`} events={r.events} />
            ) : (
              <Row key={r.e.index} e={r.e} />
            ),
          )}
          {/* Held messages sit at the tail, in the position they will
              occupy once they land. Dimmed rather than styled apart:
              they are your turns, they are just not there yet. */}
          {queued.map((m) => (
            <QueuedRow
              key={m.id}
              m={m}
              onCancel={() => cancelQueued(id, m.id)}
              onRetry={() => retryQueued(id, m.id)}
            />
          ))}
        </div>
        <div style={{ height: 'var(--s6)' }} />
      </div>

      <Composer
        text={text}
        onText={setText}
        onSend={send}
        staged={staged}
        onStage={setStaged}
        hasContent={hasContent}
        sending={sending}
        notice={notice}
        disabled={!canSend}
        holding={holding}
        queued={queued.length}
        reason={blocked}
        warning={warning}
        onPickCommand={() => setSheet('commands')}
        onPickQuick={() => setSheet('quick')}
        hasQuick={quickPrompts.length > 0}
      />

      {/* Outside the composer, deliberately. The composer sets
          `backdrop-filter`, and a filter — backdrop or otherwise —
          makes its element the containing block for `position: fixed`
          descendants. So a sheet rendered in there resolved `inset: 0`
          against a 109px-tall bar instead of the viewport and appeared
          as a strip above the tab bar. Measured, not reasoned: top 526,
          height 109, viewport 701.

          `Thread` returns a fragment, so this lands directly under
          `Shell`, which sets no filter. */}
      {sheet === 'commands' && (
        <CommandPicker
          sessionId={session.session_id}
          onStage={setStaged}
          onClose={() => setSheet(null)}
        />
      )}
      {sheet === 'quick' && (
        <QuickPicker prompts={quickPrompts} onSend={send} onClose={() => setSheet(null)} />
      )}
    </>
  );
}

function RawBanner() {
  return (
    <p
      style={{
        margin: '0 var(--gut) var(--s3)',
        padding: 'var(--s2) var(--s3)',
        borderRadius: 'var(--r-md)',
        background: 'var(--sf2)',
        color: 'var(--fg3)',
        fontSize: 'var(--t-micro)',
        lineHeight: 'var(--lh-body)',
      }}
    >
      <Ico n="shield" s="2xs" w="reg" /> Raw transcript. Known secret formats are masked; the list is
      incomplete, so treat anything here as if it were unredacted.
    </p>
  );
}

const WHO = {
  user: { label: 'You', color: 'var(--ac-ink)' },
  assistant: { label: 'Claude', color: 'var(--fg)' },
  thinking: { label: 'Thinking', color: 'var(--fg4)' },
  system: { label: 'System', color: 'var(--fg4)' },
  summary: { label: 'Summary', color: 'var(--fg4)' },
};

function Row({ e }) {
  if (e.kind === 'tool') return <ToolTick e={e} />;
  const who = WHO[e.kind] || WHO.system;
  return (
    <div style={{ padding: 'var(--s4) 0' }}>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 'var(--s2)' }}>
        <span
          style={{
            fontSize: 'var(--t-micro)',
            fontWeight: 'var(--w-bold)',
            letterSpacing: 'var(--ls-caps)',
            textTransform: 'uppercase',
            color: who.color,
          }}
        >
          {who.label}
        </span>
        <span className="mono" style={{ fontSize: 'var(--t-nano)', color: 'var(--fg4)' }}>
          {ago(e.ts)}
        </span>
      </div>
      {/* Prose is markdown; Claude writes it that way. Tool ticks are
          not — see Markdown.jsx on why rendering stdout would corrupt
          it. `thinking` keeps its quieter type by scoping a size and
          colour onto the wrapper rather than by opting out. */}
      <div
        style={{
          marginTop: 'var(--s2)',
          fontSize: e.kind === 'thinking' ? 'var(--t-meta)' : undefined,
          color: e.kind === 'thinking' ? 'var(--fg4)' : undefined,
        }}
      >
        <Markdown text={e.text} />
      </div>
    </div>
  );
}

/**
 * A message typed while the Mac was unreachable.
 *
 * Rendered where it will land, at half opacity, with the way to take it
 * back on the row itself. A queued message is going to be sent later by
 * a phone nobody is looking at; without a cancel the only way to stop it
 * is to be elsewhere when it fires.
 *
 * A refused entry stops retrying and says why — see `outbox.js` on the
 * busy loop that alternative would be — and offers the two answers a
 * person can give it.
 */
function QueuedRow({ m, onCancel, onRetry }) {
  return (
    <div style={{ padding: 'var(--s4) 0', opacity: m.failed ? 1 : 0.55 }}>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 'var(--s2)' }}>
        <span
          style={{
            fontSize: 'var(--t-micro)',
            fontWeight: 'var(--w-bold)',
            letterSpacing: 'var(--ls-caps)',
            textTransform: 'uppercase',
            color: 'var(--ac-ink)',
          }}
        >
          You
        </span>
        {m.failed ? (
          <Chip tone="danger" ico="alert" size="xs">
            not sent
          </Chip>
        ) : (
          <Chip tone="warn" ico="clock" size="xs">
            queued
          </Chip>
        )}
        <button
          onClick={onCancel}
          aria-label="Discard this queued message"
          title="Discard"
          style={{ marginLeft: 'auto', padding: 'var(--s1)', color: 'var(--fg4)' }}
        >
          <Ico n="x" s="2xs" w="reg" />
        </button>
      </div>
      <p
        style={{
          marginTop: 'var(--s2)',
          fontSize: 'var(--t-body)',
          lineHeight: 'var(--lh-body)',
          whiteSpace: 'pre-wrap',
          overflowWrap: 'anywhere',
        }}
      >
        {m.text}
      </p>
      {m.failed && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--s2)', marginTop: 'var(--s2)' }}>
          <p role="alert" style={{ flex: 1, fontSize: 'var(--t-micro)', color: 'var(--dg)' }}>
            {m.failed}
          </p>
          <Chip tone="quiet" size="xs" onClick={onRetry}>
            Try again
          </Chip>
        </div>
      )}
    </div>
  );
}

/**
 * A run of consecutive tool calls, as one row.
 *
 * Not a hide: the count stays and the row opens into exactly the ticks
 * it replaced, so a working session still looks different from a
 * stalled one. Measured, 59–91% of a real transcript is these — which
 * is why the default folds them and why the row has to stay honest
 * about how many it swallowed.
 */
function ToolGroup({ events }) {
  const [open, setOpen] = useState(false);
  const failed = events.filter((e) => e.is_error).length;
  if (open) {
    return (
      <div>
        <button
          onClick={() => setOpen(false)}
          aria-expanded="true"
          style={{ display: 'flex', alignItems: 'center', gap: 'var(--s2)', width: '100%', padding: 'var(--s1) 0' }}
        >
          <Ico n="chevD" s="2xs" w="thin" c="var(--fg4)" />
          <span style={{ fontSize: 'var(--t-micro)', color: 'var(--fg4)' }}>
            {events.length} tool calls
          </span>
        </button>
        {events.map((e) => (
          <ToolTick key={e.index} e={e} />
        ))}
      </div>
    );
  }
  return (
    <div style={{ padding: 'var(--s1) 0' }}>
      <button
        onClick={() => setOpen(true)}
        aria-expanded="false"
        style={{ display: 'flex', alignItems: 'center', gap: 'var(--s2)', width: '100%', textAlign: 'left' }}
      >
        <Ico n="chevR" s="2xs" w="thin" c="var(--fg4)" />
        <span style={{ fontSize: 'var(--t-micro)', color: 'var(--fg4)' }}>
          {events.length} tool calls
        </span>
        {/* An errored call must survive the fold, or grouping hides the
            one thing in a run worth looking at. */}
        {failed > 0 && (
          <span style={{ fontSize: 'var(--t-nano)', color: 'var(--dg)' }}>
            {failed} errored
          </span>
        )}
      </button>
    </div>
  );
}

function ToolTick({ e }) {
  const [open, setOpen] = useState(false);
  return (
    <div style={{ padding: 'var(--s1) 0' }}>
      <button
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 'var(--s2)',
          width: '100%',
          textAlign: 'left',
        }}
      >
        <Ico n={open ? 'chevD' : 'chevR'} s="2xs" w="thin" c="var(--fg4)" />
        <span
          className="mono"
          style={{
            fontSize: 'var(--t-micro)',
            color: e.is_error ? 'var(--dg)' : 'var(--fg3)',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {e.tool_name}
          {e.text ? ` ${e.text}` : ''}
        </span>
      </button>
      {open && e.detail && (
        <pre
          className="mono"
          style={{
            marginTop: 'var(--s2)',
            padding: 'var(--s3)',
            borderRadius: 'var(--r-md)',
            background: 'var(--sf2)',
            fontSize: 'var(--t-nano)',
            color: 'var(--fg2)',
            whiteSpace: 'pre-wrap',
            overflowWrap: 'anywhere',
          }}
        >
          {e.detail}
        </pre>
      )}
    </div>
  );
}

function Composer({
  text,
  onText,
  onSend,
  sending,
  notice,
  disabled,
  holding,
  queued,
  reason,
  warning,
  staged,
  onStage,
  hasContent,
  onPickCommand,
  onPickQuick,
  hasQuick,
}) {

  return (
    <div
      style={{
        flexShrink: 0,
        // No `--safe-b` here: the tab bar sits below the composer and
        // owns the home-indicator inset. Both claiming it left a strip
        // of dead space above the bar.
        padding: 'var(--s3) var(--gut)',
        background: 'var(--glass)',
        backdropFilter: 'blur(var(--blur-bar))',
        WebkitBackdropFilter: 'blur(var(--blur-bar))',
        boxShadow: 'inset 0 var(--bw-hair) 0 var(--hair)',
      }}
    >
      {/* The count, not just the state. "Offline" is something the user
          already knows by the time they have typed two messages into
          it; how many are waiting is the part they cannot see. */}
      {holding && queued > 0 && (
        <p
          role="status"
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 'var(--s2)',
            fontSize: 'var(--t-micro)',
            color: 'var(--wn)',
            marginBottom: 'var(--s2)',
          }}
        >
          <Ico n="clock" s="2xs" w="bold" />
          {queued} held — sends when this Mac is back
        </p>
      )}
      {notice && (
        <p
          role="status"
          style={{ fontSize: 'var(--t-micro)', color: 'var(--fg3)', marginBottom: 'var(--s2)' }}
        >
          {notice}
        </p>
      )}
      {warning && (
        <p
          role="status"
          style={{ fontSize: 'var(--t-micro)', color: 'var(--wn)', marginBottom: 'var(--s2)' }}
        >
          <Ico n="alert" s="2xs" w="bold" /> {warning}
        </p>
      )}
      {/* The staged expansion. A chip rather than 14,000 characters in
          the textarea — same commitment, since nothing is sent until
          Send, without making the composer impossible to scroll past. */}
      {staged && (
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 'var(--s2)',
            marginBottom: 'var(--s2)',
            padding: 'var(--s2) var(--s3)',
            borderRadius: 'var(--r-md)',
            background: 'var(--ac-wash)',
          }}
        >
          <Ico n="file" s="2xs" c="var(--ac-ink)" />
          <span className="mono" style={{ fontSize: 'var(--t-micro)', color: 'var(--ac-ink)' }}>
            /{staged.name}
          </span>
          <span style={{ fontSize: 'var(--t-nano)', color: 'var(--fg4)' }}>
            {staged.chars < 1000 ? staged.chars : `${(staged.chars / 1000).toFixed(1)}k`} chars
            {staged.restricts_tools ? ' · tool limits not carried' : ''}
          </span>
          <button
            onClick={() => onStage(null)}
            aria-label={`Remove ${staged.name}`}
            style={{ marginLeft: 'auto', padding: 'var(--s1)' }}
          >
            <Ico n="x" s="2xs" c="var(--fg3)" />
          </button>
        </div>
      )}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          onSend();
        }}
        style={{ display: 'flex', gap: 'var(--s2)', alignItems: 'flex-end' }}
      >
        {/* Before `/`, and only when there is something behind it.
            These used to be a scrolling row of chips above the
            composer, which cost a line on every thread and showed about
            four before running off the edge. Behind a sheet the list is
            as long as its owner wants and is searchable. */}
        {hasQuick && (
          <button
            type="button"
            onClick={onPickQuick}
            disabled={disabled}
            aria-label="Send a quick prompt"
            title="Send a quick prompt"
            style={{
              display: 'grid',
              placeItems: 'center',
              height: 'var(--ctl-lg)',
              width: 'var(--ctl-lg)',
              flexShrink: 0,
              borderRadius: 'var(--r-lg)',
              background: 'var(--sf2)',
              color: 'var(--fg3)',
              fontSize: 'var(--t-body)',
            }}
          >
            …
          </button>
        )}
        <button
          type="button"
          onClick={onPickCommand}
          disabled={disabled}
          aria-label="Insert a command"
          title="Insert a command"
          style={{
            // `grid`/`place-items`, not the button default: the design
            // system's reset makes every button `display: block` with
            // `text-align: left`, so the glyph sat flush against the
            // left edge of its own circle. Measured — box left 22,
            // glyph left 22.
            display: 'grid',
            placeItems: 'center',
            height: 'var(--ctl-lg)',
            width: 'var(--ctl-lg)',
            flexShrink: 0,
            borderRadius: 'var(--r-lg)',
            background: 'var(--sf2)',
            color: 'var(--fg3)',
            fontFamily: 'var(--f-mono)',
            fontSize: 'var(--t-body)',
          }}
        >
          /
        </button>
        <textarea
          value={text}
          onChange={(e) => onText(e.target.value)}
          rows={1}
          disabled={disabled}
          aria-label="Message this session"
          // Short enough not to wrap. The `/` button costs 54px, which
          // left the field 184px at 390 — and "Message this session…"
          // wrapped to a second line the one-row height then clipped.
          placeholder={
            disabled
              ? reason || 'Unavailable'
              : staged
                ? 'Add a note…'
                : holding
                  ? 'Queue a message…'
                  : 'Message this session…'
          }
          style={{
            flex: 1,
            minHeight: 'var(--ctl-lg)',
            maxHeight: 'calc(var(--ctl-lg) * 4)',
            padding: 'var(--s3) var(--s4)',
            borderRadius: 'var(--r-lg)',
            background: 'var(--sf2)',
            color: 'var(--fg)',
            fontFamily: 'inherit',
            // 16px is the floor that stops iOS zooming on focus.
            fontSize: 'var(--t-body)',
            lineHeight: 'var(--lh-body)',
            resize: 'none',
          }}
        />
        {/* Icon-only, and the one place in this app that overrides
            `rules/icon-buttons.md`'s "never the primary action". An
            upward arrow in a message composer is as close to a
            universal verb as this interface has — iMessage, WhatsApp,
            Telegram and ChatGPT all ship it unlabelled — so the rule's
            actual test, "can the user scan the surface and know what it
            does without reading", is met. It also gives ~55px back to a
            390px row that the `/` button had taken. */}
        {/* A clock, not an arrow, while the Mac is unreachable. The
            glyph is the honest one: this press does not send, it holds
            — and the label says so for anyone who cannot see it. */}
        <Btn
          kind="primary"
          disabled={disabled || sending || !hasContent}
          ico={sending ? undefined : holding ? 'clock' : 'up'}
          aria-label={holding ? 'Hold until this Mac is back' : 'Send'}
          style={{ width: 'var(--ctl-lg)', padding: 0, flexShrink: 0 }}
        >
          {sending ? '…' : ''}
        </Btn>
      </form>

      {disabled && reason && (
        <p style={{ fontSize: 'var(--t-micro)', color: 'var(--fg4)', marginTop: 'var(--s2)' }}>
          {reason}
        </p>
      )}
    </div>
  );
}
