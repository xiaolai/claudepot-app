// The hub: a greeting, the two or three things that actually want you,
// then history. Home is a list, not a dashboard.
import { useState } from 'react';

import { api, explainSend, newIdempotencyKey } from './api.js';
import { ago, basename, compact, modelLabel, span } from './format.js';

const { Badge, Chip, Dot, Group, Ico, Item, List, Surface } = window;

/**
 * Badge text. Past two digits the exact number stops being information
 * and starts being a wide pill — a session left alone over a weekend
 * accrues hundreds of events, and "how many" is not the question the
 * badge answers.
 */
function badgeCount(n) {
  if (!n) return null;
  return n > 99 ? '99+' : n;
}

function greet(now = new Date()) {
  const h = now.getHours();
  if (h < 5) return 'Still up';
  if (h < 12) return 'Good morning';
  if (h < 18) return 'Good afternoon';
  return 'Good evening';
}

/**
 * Card tone. Four in the design system; three are reachable.
 *
 * `failed` is not synthesised. The only signal available is
 * `SessionRow::has_error`, which is true of any transcript containing a
 * single errored tool call — routine in a long session. Painting those
 * red would make the one colour that should mean "look at this" mean
 * "this session ran a command that exited non-zero", which is most of
 * them. `has_error` is rendered as a quiet marker instead.
 */
function tone(s) {
  if (s.status === 'waiting') return 'waiting';
  if (s.live) return 'live';
  return 'done';
}

const TONE_COLOR = {
  live: 'var(--ac)',
  waiting: 'var(--wn)',
  done: 'var(--fg4)',
};

export function Sessions({ sessions, conn, host, onOpen, onRetry, onChanged }) {
  if (sessions === null) {
    return (
      <Scroll>
        <Header host={host} />
        <p style={{ color: 'var(--fg4)', fontSize: 'var(--t-meta)', padding: '0 var(--s1)' }}>Loading…</p>
      </Scroll>
    );
  }

  const attention = sessions.filter((s) => s.live);
  const history = sessions.filter((s) => !s.live);

  return (
    <Scroll>
      <Header host={host} />

      {attention.length === 0 && history.length === 0 && (
        <Surface>
          <p style={{ fontSize: 'var(--t-sub)', color: 'var(--fg2)' }}>
            No Claude Code sessions on this Mac yet.
          </p>
          <p style={{ fontSize: 'var(--t-meta)', color: 'var(--fg4)', marginTop: 'var(--s2)' }}>
            Start one at the machine and it will appear here.
          </p>
        </Surface>
      )}

      {attention.length > 0 && (
        <Group title="Now">
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s3)' }}>
            {attention.map((s) => (
              <LiveCard key={s.session_id} s={s} onOpen={onOpen} onChanged={onChanged} conn={conn} />
            ))}
          </div>
        </Group>
      )}

      {history.length > 0 && (
        <Group title="Earlier">
          <List>
            {history.map((s, i) => (
              <Item key={s.session_id} first={i === 0} onClick={() => onOpen(s.session_id)}>
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
                    {s.title || 'Untitled session'}
                  </div>
                  <Meta s={s} />
                </div>
                {s.unread > 0 && <Badge n={badgeCount(s.unread)} tone="accent" />}
                <Ico n="chevR" s="sm" w="thin" c="var(--fg4)" />
              </Item>
            ))}
          </List>
        </Group>
      )}

      {conn === 'offline' && (
        <p style={{ marginTop: 'var(--s5)', color: 'var(--fg4)', fontSize: 'var(--t-micro)' }}>
          Showing the last list received.{' '}
          <button onClick={onRetry} style={{ color: 'var(--ac-ink)', fontWeight: 'var(--w-semi)' }}>
            Retry
          </button>
        </p>
      )}
    </Scroll>
  );
}

function Header({ host }) {
  return (
    <header style={{ padding: 'var(--s6) 0 var(--s2)' }}>
      <h1 className="disp" style={{ fontSize: 'var(--t-hero)' }}>
        {greet()}
      </h1>
      {host && (
        <p className="mono" style={{ fontSize: 'var(--t-micro)', color: 'var(--fg4)', marginTop: 'var(--s1)' }}>
          {host}
        </p>
      )}
    </header>
  );
}

/**
 * The dense line under a session title.
 *
 * State B under `rules/path-display.md`: the project path is truncated
 * to its basename with the full string in `title`. The canonical copy
 * site is the thread header — `Thread`'s `<CopyPath>` — which every row
 * opens. Do not add a copy button per row; at 28 rows it is noise, and
 * that is the trade the rule names.
 *
 * `showProject` is false on a live card, where the header already names
 * the project. Saying it twice in six inches of screen is worse than
 * either place saying it once.
 */
function Meta({ s, showProject = true }) {
  const bits = [];
  if (s.branch) bits.push(s.branch);
  if (s.messages) bits.push(`${s.messages} msg`);
  // Input + output only. The server sends all four components because
  // which of them belongs on a card is a presentation call, and cache
  // reads — two orders of magnitude larger on a long session — are not
  // what "how big is this conversation" means.
  const billable = (s.tokens?.input ?? 0) + (s.tokens?.output ?? 0);
  if (billable) bits.push(`${compact(billable)} tok`);
  if (s.models?.length === 1) bits.push(modelLabel(s.models[0]));
  else if (s.models?.length > 1) bits.push(`${s.models.length} models`);
  const when = ago(s.last_ts);
  if (when !== '—') bits.push(when);
  const d = span(s.first_ts, s.last_ts);
  if (d) bits.push(d);

  return (
    <div
      className="mono"
      style={{
        display: 'flex',
        gap: 'var(--s2)',
        fontSize: 'var(--t-micro)',
        color: 'var(--fg4)',
        marginTop: 'var(--s1)',
        overflow: 'hidden',
        whiteSpace: 'nowrap',
      }}
      title={s.project_path || undefined}
    >
      {showProject && <span style={{ color: 'var(--fg3)' }}>{basename(s.project_path)}</span>}
      {bits.map((b, i) => (
        <span key={i}>
          {showProject || i > 0 ? '· ' : ''}
          {b}
        </span>
      ))}
      {s.has_error && <span style={{ color: 'var(--wn)' }}>· had errors</span>}
    </div>
  );
}

/**
 * The status, as a mark rather than a word.
 *
 * Shape and motion carry it, not colour: `rules/design.md`'s
 * accessibility floor says colour never carries meaning alone, and three
 * dots that differ only in hue would be exactly that. Waiting gets its
 * own glyph, busy pulses, attached sits still — distinguishable in
 * greyscale and to anyone who cannot see the card at all, via the label.
 */
function StatusMark({ s }) {
  const label =
    s.status === 'waiting' ? 'Waiting for you' : s.status === 'busy' ? 'Live' : 'Attached';
  return (
    <span role="img" aria-label={label} title={label} style={{ display: 'grid', placeItems: 'center' }}>
      {s.status === 'waiting' ? (
        <Ico n="alert" s="sm" w="bold" c={TONE_COLOR.waiting} />
      ) : (
        <Dot c={TONE_COLOR[tone(s)]} pulse={s.status === 'busy'} size="md" />
      )}
    </span>
  );
}

function LiveCard({ s, onOpen, onChanged, conn }) {
  const [sending, setSending] = useState(null);
  const [sent, setSent] = useState(null);
  const [failed, setFailed] = useState(null);
  const t = tone(s);

  const reply = async (choice) => {
    if (sending || conn === 'offline') return;
    setSending(choice);
    setFailed(null);
    try {
      await api.sendPrompt(s.session_id, choice, newIdempotencyKey());
      setSent(choice);
      onChanged?.();
    } catch (e) {
      // A tap that quietly did nothing is the worst outcome on this
      // surface: the user believes they answered and walks away. The
      // wording is shared with the thread composer so the same failure
      // does not get two different explanations.
      setSent(null);
      setFailed(explainSend(e));
    } finally {
      setSending(null);
    }
  };

  return (
    <Surface glow={t === 'live' && s.status === 'busy'}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--s2)', marginBottom: 'var(--s3)' }}>
        <StatusMark s={s} />
        {/* The project, where the status word used to be. "LIVE" and
            "ATTACHED" said what the mark beside them already says; the
            project is the thing that tells two live sessions apart. */}
        <span
          className="mono"
          title={s.project_path || undefined}
          style={{
            fontSize: 'var(--t-micro)',
            fontWeight: 'var(--w-bold)',
            letterSpacing: 'var(--ls-caps)',
            color: 'var(--fg3)',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {basename(s.project_path)}
        </span>
        <span style={{ flex: 1 }} />
        {s.unread > 0 && <Badge n={badgeCount(s.unread)} />}
      </div>

      <button
        onClick={() => onOpen(s.session_id)}
        style={{ display: 'block', width: '100%', textAlign: 'left' }}
      >
        <div style={{ fontSize: 'var(--t-head)', fontWeight: 'var(--w-semi)', lineHeight: 'var(--lh-tight)' }}>
          {s.title || 'Untitled session'}
        </div>
        <Meta s={s} showProject={false} />
      </button>

      {s.last_line && (
        <p
          style={{
            marginTop: 'var(--s3)',
            fontSize: 'var(--t-meta)',
            color: 'var(--fg2)',
            lineHeight: 'var(--lh-body)',
            display: '-webkit-box',
            WebkitLineClamp: 3,
            WebkitBoxOrient: 'vertical',
            overflow: 'hidden',
          }}
        >
          {s.last_line}
        </p>
      )}

      {s.ask && (
        <Ask
          ask={s.ask}
          sending={sending}
          sent={sent}
          failed={failed}
          onReply={reply}
          disabled={conn === 'offline' || !s.addressable}
        />
      )}

      {!s.ask && s.waiting_for && (
        <p style={{ marginTop: 'var(--s3)', fontSize: 'var(--t-meta)', color: 'var(--wn)' }}>
          <Ico n="alert" s="2xs" w="bold" /> Waiting to {s.waiting_for} — answer at the machine.
        </p>
      )}
    </Surface>
  );
}

/**
 * The design's best idea: a waiting session offers Claude's own choices
 * as one-tap chips.
 *
 * Tapping one sends the option's label as a *prompt*, which is all this
 * channel can carry. Claude Code wraps a peer message and may hold it
 * for the local user's approval, so the wording after a tap is "handed
 * off", never "sent".
 */
function Ask({ ask, sending, sent, failed, onReply, disabled }) {
  const caveats = [];
  if (ask.more_questions > 0) {
    caveats.push(
      `Claude asked ${ask.more_questions + 1} questions — one tap answers this one only.`,
    );
  }
  if (ask.options_omitted > 0) {
    caveats.push(`${ask.options_omitted} more choices are only on screen at the machine.`);
  }
  return (
    <div style={{ marginTop: 'var(--s4)' }}>
      <p style={{ fontSize: 'var(--t-meta)', color: 'var(--fg)', lineHeight: 'var(--lh-body)' }}>{ask.question}</p>
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 'var(--s2)', marginTop: 'var(--s3)' }}>
        {ask.options.map((o) => (
          <Chip
            key={o.label}
            tone={sent === o.label ? 'ok' : 'accent'}
            size="md"
            onClick={disabled ? undefined : () => onReply(o.label)}
          >
            {sending === o.label ? '…' : o.label}
          </Chip>
        ))}
      </div>
      {caveats.map((c) => (
        <p key={c} style={{ marginTop: 'var(--s2)', fontSize: 'var(--t-micro)', color: 'var(--fg4)' }}>
          {c}
        </p>
      ))}
      {failed && (
        <p role="alert" style={{ marginTop: 'var(--s2)', fontSize: 'var(--t-micro)', color: 'var(--dg)' }}>
          {failed}
        </p>
      )}
      {sent && !failed && (
        <p role="status" style={{ marginTop: 'var(--s2)', fontSize: 'var(--t-micro)', color: 'var(--fg4)' }}>
          Handed off as a message. Claude Code may hold it for approval at the machine, and the
          question stays here until the transcript says it was settled.
        </p>
      )}
    </div>
  );
}

function Scroll({ children }) {
  return (
    <div className="sc" style={{ flex: 1, minHeight: 0, padding: '0 var(--gut) var(--s8)' }}>
      {children}
    </div>
  );
}
