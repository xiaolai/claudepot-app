// Presentation helpers.
//
// The server sends instants (RFC 3339) and counts; the phone renders
// "12m" and "1h 08m". Formatting client-side is not a stylistic choice —
// a relative time computed on the host is stale the moment it is
// serialised, and a panel left open on a bedside table would show "12m"
// an hour later.

/** `2026-08-23T01:12:00Z` → `12m`, `3h`, `2d`. `null` → `—`. */
export function ago(iso, now = Date.now()) {
  if (!iso) return '—';
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return '—';
  return short(Math.max(0, now - t));
}

/** Duration between two instants, in the same compact register. */
export function span(fromIso, toIso) {
  if (!fromIso || !toIso) return null;
  const a = Date.parse(fromIso);
  const b = Date.parse(toIso);
  if (Number.isNaN(a) || Number.isNaN(b) || b < a) return null;
  return long(b - a);
}

/**
 * How long until an instant, or `null` if it has passed.
 *
 * `ago` clamps at zero, so feeding it a future date returns "0s" — which
 * is why the session-expiry row needs its own helper rather than a
 * string replace on `ago`'s output.
 */
export function until(iso, now = Date.now()) {
  if (!iso) return null;
  const t = Date.parse(iso);
  if (Number.isNaN(t) || t <= now) return null;
  return short(t - now);
}

/** `12m`, `3h`, `2d` — one unit, for "how long ago". */
export function short(ms) {
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h`;
  return `${Math.floor(h / 24)}d`;
}

/** `14m`, `1h 08m` — two units above an hour, for "how long it took". */
export function long(ms) {
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  return `${h}h ${String(m % 60).padStart(2, '0')}m`;
}

/** 84120 → `84.1k`. Tokens are compared, not audited, on a phone. */
export function compact(n) {
  if (n === null || n === undefined) return null;
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

/** Basename of a path, native separator either way. */
export function basename(path) {
  if (!path) return '';
  const parts = String(path).split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] || path;
}

/**
 * Shorten a path for a narrow column, keeping the basename.
 *
 * Paths read from the end — the basename is the identity, the prefix is
 * shared context. The full string always travels alongside as a `title`
 * so nothing that is clipped becomes unreadable.
 */
export function tightPath(path, maxChars = 34) {
  if (!path) return '';
  const s = String(path);
  if (s.length <= maxChars) return s;
  const base = basename(s);
  if (base.length + 2 >= maxChars) return `…${base.slice(-(maxChars - 1))}`;
  return `…${s.slice(-(maxChars - 1))}`;
}

/** One line of a model id, short enough for a card. */
export function modelLabel(model) {
  if (!model) return null;
  return String(model)
    .replace(/^claude-/, '')
    .replace(/-\d{8}$/, '');
}

/** `842 MB`, `3.1 GB`. Binary units, because that is what a filesystem reports. */
export function bytes(n) {
  if (n === null || n === undefined) return null;
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(0)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(1)} GB`;
}

/**
 * Fold consecutive tool ticks into one row.
 *
 * Returns a list of `{kind: 'one', e}` and `{kind: 'group', events}`.
 *
 * A run of **one** is left alone deliberately: collapsing a single tick
 * into a row reading "1 tool call" is strictly worse than the tick,
 * which at least names the tool. The saving starts at two.
 *
 * Order is never changed and nothing is dropped — every input event
 * appears exactly once in the output, so a group can always be expanded
 * back into precisely what it replaced.
 */
export function groupTools(events, mode = 'grouped') {
  if (mode === 'shown') return events.map((e) => ({ kind: 'one', e }));
  const out = [];
  let run = [];
  const flush = () => {
    if (run.length === 0) return;
    if (run.length === 1) out.push({ kind: 'one', e: run[0] });
    else out.push({ kind: 'group', events: run });
    run = [];
  };
  for (const e of events) {
    if (e.kind === 'tool') run.push(e);
    else {
      flush();
      out.push({ kind: 'one', e });
    }
  }
  flush();
  return out;
}

// A chip per row only where something HAPPENED. `null` means "say
// nothing", and both silent entries are deliberate: `ok` is the
// expected state, and `never` is an absence of information rather than
// a finding — badging every row with "we haven't looked" is the same
// noise as rendering `0 sessions`.
export const VERIFY_TONE = {
  ok: null,
  never: null,
  drift: { tone: 'warn', label: 'Drift' },
  rejected: { tone: 'danger', label: 'Rejected' },
  signed_out: { tone: 'danger', label: 'Signed out' },
  network_error: { tone: 'quiet', label: 'Unverified' },
};

/**
 * The verify chip for a status, or nothing.
 *
 * Not `VERIFY_TONE[status] ?? VERIFY_TONE.never`: `??` cannot tell a
 * deliberate `null` from a missing key, so it substituted the fallback
 * for every entry meaning "say nothing" — and every *verified* account
 * was labelled "Never verified", the opposite of the truth, on every
 * row.
 *
 * With `never` also silent the two forms happen to agree today, which
 * is exactly why this is written the honest way: the next label added
 * to `never` would silently resurrect the bug, and the tests below fail
 * on that combination rather than on this line alone.
 */
export function verifyChip(status) {
  return status in VERIFY_TONE ? VERIFY_TONE[status] : VERIFY_TONE.never;
}

/**
 * When a usage window rolls over, in the reader's own timezone.
 *
 * Absolute rather than relative ("28 Aug 09:00", not "in 4d"): a
 * relative figure answers how long you must wait, and the question in
 * front of an exhausted week is *when can I start again* — which is a
 * time you can plan around. Same-day resets drop the date, because on
 * a 5-hour window the date is noise and only the clock matters.
 *
 * `null` for a window the server gave no timestamp: it returns one only
 * once a window has activity, so a meter at 0% has nothing to reset and
 * says nothing rather than inventing a date.
 */
export function resetAt(iso, now = new Date()) {
  if (!iso) return null;
  const d = new Date(iso);
  if (!Number.isFinite(d.getTime())) return null;
  const hm = d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false });
  if (d.toDateString() === now.toDateString()) return hm;
  return `${d.toLocaleDateString([], { day: 'numeric', month: 'short' })} ${hm}`;
}
