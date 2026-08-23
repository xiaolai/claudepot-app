// The two stateful jobs `Thread` still had beyond rendering.
//
// Kept out of the component for the reason the transcript hook was: the
// bug that lived in the read mark — a high-water ref that survived a
// session switch, so opening a short session after a long one never
// cleared its badge — was invisible inside a function that also did
// scroll tracking, sending, paging and layout.
import { useCallback, useEffect, useRef, useState } from 'react';

import { api, explainSend, newIdempotencyKey } from './api.js';

/** px from the bottom that still counts as "following along". */
export const AT_END_SLOP = 90;

/**
 * Follow the tail, and clear the badge once the newest event is on it.
 *
 * The two are one hook because they answer the same question — is the
 * user actually looking at the end? — and splitting them would mean two
 * copies of `atEnd`.
 */
export function useFollowTail(sessionId, total, events) {
  const scroller = useRef(null);
  const atEnd = useRef(true);
  // Per session. A ref survives the switch, so this must be reset with
  // `sessionId` or a 40-event session opened after a 1600-event one
  // finds the mark already above its total and is never marked read.
  const marked = useRef(0);
  // The effect below and the scroll handler both need the freshest
  // `total`, and the handler is memoised with an empty dependency list
  // so it can be handed to a DOM listener without re-binding.
  const latestTotal = useRef(total);
  latestTotal.current = total;
  const latestId = useRef(sessionId);
  latestId.current = sessionId;

  useEffect(() => {
    atEnd.current = true;
    marked.current = 0;
  }, [sessionId]);

  useEffect(() => {
    const el = scroller.current;
    if (el && atEnd.current) el.scrollTop = el.scrollHeight;
  }, [events]);

  /**
   * Record the read mark, if the user is actually looking at the tail.
   *
   * Called from two places on purpose. The effect covers "new events
   * arrived while you were at the bottom"; the scroll handler covers
   * "you scrolled back down to events that were already here". Without
   * the second, a badge stayed lit after the user had plainly read the
   * message — `atEnd` is a ref, so returning to the bottom re-renders
   * nothing and no effect fires.
   */
  const markIfAtEnd = useCallback(() => {
    const count = latestTotal.current;
    if (count === null || count === undefined) return;
    if (!atEnd.current || count <= marked.current) return;
    marked.current = count;
    const id = latestId.current;
    // `total` is a COUNT of events, which is what the server stores.
    api.markRead(id, count, newIdempotencyKey()).catch(() => {
      // A badge that stays lit is a cosmetic failure; retrying on the
      // next append or the next scroll is the recovery.
      if (latestId.current === id) marked.current = 0;
    });
  }, []);

  useEffect(() => {
    markIfAtEnd();
  }, [sessionId, total, events, markIfAtEnd]);

  const onScroll = useCallback(
    (e) => {
      const el = e.currentTarget;
      const wasAtEnd = atEnd.current;
      atEnd.current = el.scrollHeight - el.scrollTop - el.clientHeight < AT_END_SLOP;
      if (!wasAtEnd && atEnd.current) markIfAtEnd();
    },
    [markIfAtEnd],
  );

  return { scroller, onScroll };
}

/**
 * Text that Claude Code will treat as a slash command if typed at the
 * terminal, and as literal prose if it arrives over the peer socket.
 *
 * A word, not a path. `/compact` matches; `/Users/joker/x` does not,
 * because a second slash means the user is talking about a file.
 */
const LOOKS_LIKE_SLASH_COMMAND = /^\/[a-z][a-z0-9:_-]*(\s|$)/i;

/**
 * Handing a prompt to a session.
 *
 * `canSend` and `blocked` travel together because a disabled control
 * that does not say why is the thing `rules/design.md` forbids: "disabled
 * buttons state a reason inline".
 *
 * `warning` is the softer case: the send will succeed and do something
 * other than what the user meant. Claude Code dispatches an injected
 * prompt with `skipSlashCommands: true` — its own predicate for "is this
 * a command" is `startsWith("/") && !skipSlashCommands` — so `/compact`
 * arrives as four-and-a-bit characters of text and Claude replies about
 * it instead of running it. Verified against the 2.1.241 binary.
 *
 * A warning rather than a block: `/` also starts a path, and refusing to
 * send text the user meant to send would be worse than telling them what
 * will happen to it.
 */
export function useSendPrompt(session, conn, onChanged) {
  const [text, setText] = useState('');
  const [sending, setSending] = useState(false);
  const [notice, setNotice] = useState(null);
  // An expanded slash command, staged but not sent.
  //
  // Staged rather than pasted into the textarea, though "insert into
  // the composer" is what this is: `cc-suite:audit-fix` expands to
  // 14,208 characters, and a phone textarea holding that is a textarea
  // you cannot scroll past to reach the send button. The chip is the
  // same commitment — nothing leaves until send — without making the
  // composer unusable to get there.
  const [staged, setStaged] = useState(null);

  const canSend = Boolean(session.live && session.addressable && conn !== 'offline');
  // The warning is pointless once a command is staged — that is the
  // supported way to send one, and repeating "slash commands do not
  // run" over a staged expansion would contradict the chip above it.
  const warning =
    !staged && LOOKS_LIKE_SLASH_COMMAND.test(text.trim())
      ? 'Slash commands do not run over this channel — pick one with / to send its text instead.'
      : null;
  const blocked = !session.live
    ? 'This session is not running — nothing to send to.'
    : !session.addressable
      ? 'This session has no message inbox — an older Claude Code, or its socket did not bind.'
      : conn === 'offline'
        ? 'Offline.'
        : null;

  const send = useCallback(
    async (body) => {
      // A staged command leads; anything typed follows it as a note.
      // The other order buries the instruction under the aside.
      const typed = (body ?? text).trim();
      const value = staged ? [staged.text, typed].filter(Boolean).join('\n\n') : typed;
      if (!value || sending || !canSend) return;
      setSending(true);
      setNotice(null);
      try {
        await api.sendPrompt(session.session_id, value, newIdempotencyKey());
        setText('');
        setStaged(null);
        setNotice('Handed off. Claude Code may hold it for approval at the machine.');
        onChanged?.();
      } catch (e) {
        // Deliberately NOT cleared on failure: the expansion took a
        // round trip to fetch, and dropping it would make a retry mean
        // finding the command again.
        setNotice(explainSend(e));
      } finally {
        setSending(false);
      }
    },
    [text, staged, sending, canSend, session.session_id, onChanged],
  );

  return {
    text,
    setText,
    sending,
    notice,
    send,
    canSend,
    blocked,
    warning,
    staged,
    setStaged,
    // Something to send if either half is present.
    hasContent: Boolean(staged || text.trim()),
  };
}
