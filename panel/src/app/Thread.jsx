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
import { useState } from 'react';

import { ago, modelLabel, tightPath } from './format.js';
import { Markdown } from './Markdown.jsx';
import { useTranscript } from './useTranscript.js';
import { useFollowTail, useSendPrompt } from './useThreadState.js';
import { CopyPath, Muted } from './views.jsx';

const { Btn, Chip, Dot, Ico, Tap } = window;

const INTENTS = ['Continue', 'Explain that', 'Run the tests', 'Show me the diff'];

export function Thread({ session, onBack, onChanged, conn }) {
  const id = session.session_id;
  const { events, total, loading, error, hasEarlier, loadEarlier } = useTranscript(id);
  const { scroller, onScroll } = useFollowTail(id, total, events);
  const { text, setText, sending, notice, send, canSend, blocked } = useSendPrompt(
    session,
    conn,
    onChanged,
  );

  return (
    <>
      <header
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 'var(--s2)',
          height: 'var(--bar-h-back)',
          flexShrink: 0,
          padding: '0 var(--s3) 0 var(--chev-inset)',
        }}
      >
        <Tap n="chevL" onClick={onBack} label="Back" />
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
          {events.map((e) => (
            <Row key={e.index} e={e} />
          ))}
        </div>
        <div style={{ height: 'var(--s6)' }} />
      </div>

      <Composer
        text={text}
        onText={setText}
        onSend={send}
        sending={sending}
        notice={notice}
        disabled={!canSend}
        reason={blocked}
      />
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

function Composer({ text, onText, onSend, sending, notice, disabled, reason }) {
  return (
    <div
      style={{
        flexShrink: 0,
        padding: 'var(--s3) var(--gut) var(--safe-b)',
        background: 'var(--glass)',
        backdropFilter: 'blur(var(--blur-bar))',
        WebkitBackdropFilter: 'blur(var(--blur-bar))',
        boxShadow: 'inset 0 var(--bw-hair) 0 var(--hair)',
      }}
    >
      {notice && (
        <p
          role="status"
          style={{ fontSize: 'var(--t-micro)', color: 'var(--fg3)', marginBottom: 'var(--s2)' }}
        >
          {notice}
        </p>
      )}
      {!disabled && (
        <div
          style={{ display: 'flex', gap: 'var(--s2)', overflowX: 'auto', paddingBottom: 'var(--s2)' }}
        >
          {INTENTS.map((i) => (
            <Chip key={i} tone="quiet" onClick={() => onSend(i)}>
              {i}
            </Chip>
          ))}
        </div>
      )}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          onSend();
        }}
        style={{ display: 'flex', gap: 'var(--s2)', alignItems: 'flex-end' }}
      >
        <textarea
          value={text}
          onChange={(e) => onText(e.target.value)}
          rows={1}
          disabled={disabled}
          aria-label="Message this session"
          placeholder={disabled ? reason || 'Unavailable' : 'Message this session…'}
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
        <Btn kind="primary" disabled={disabled || sending || !text.trim()} ico="arrowR">
          {sending ? '…' : 'Send'}
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
