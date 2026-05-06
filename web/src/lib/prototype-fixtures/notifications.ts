/**
 * Mock notification feed for prototype views. Different users see
 * different notifications — vary deterministically off the username.
 */

import type { Notification } from "./types";

export function getNotificationsForUser(username: string): Notification[] {
  const base: Notification[] = [
    {
      id: "n1",
      kind: "reply",
      body: "kai replied to your comment on Opus 4.7",
      link: "/post/1",
      unread: true,
      at: "2026-04-29T11:30:00Z",
    },
    {
      id: "n2",
      kind: "mention",
      body: "miro mentioned you on \"What's missing from Claude Code\"",
      link: "/post/36",
      unread: true,
      at: "2026-04-29T08:00:00Z",
    },
    {
      id: "n3",
      kind: "milestone",
      body: "Your post \"Building an eval harness\" passed 250 upvotes",
      link: "/post/2",
      unread: true,
      at: "2026-04-28T20:14:00Z",
    },
    {
      id: "n4",
      kind: "reply",
      body: "lin replied on \"one big agent or many small\"",
      link: "/post/8",
      unread: false,
      at: "2026-04-28T14:55:00Z",
    },
    {
      id: "n5",
      kind: "system",
      body: "Welcome — you have 3 unread notifications",
      link: "#",
      unread: false,
      at: "2026-04-28T09:00:00Z",
    },
  ];
  const offset = username.charCodeAt(0) % 3;
  return base.slice(offset).map((n, i) => ({ ...n, id: `${username}-${i}` }));
}

export function unreadNotificationCount(username: string): number {
  return getNotificationsForUser(username).filter((n) => n.unread).length;
}
