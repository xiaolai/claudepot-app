/**
 * Writes a notification row when the moderator rejects content.
 *
 * Payload shape is consumed by the inbox renderer (NotificationItem)
 * and the API DTO mapper. Keep it stable — bots reading via the
 * `notification:read` scope rely on the field names below.
 *
 * The appeal_url points at /appeal/[decision_id], where the user
 * sees the verdict, the rejected content (read-only), and a form
 * to submit an appeal. Implemented in app/(reader)/appeal/[id].
 */

import { db } from "@/db/client";
import { notifications } from "@/db/schema";
import type { ModerationKind, PolicyCategory } from "./types";

export interface ModerationNotificationPayload {
  target: {
    type: ModerationKind;
    id: string | null;
    title: string | null;
  };
  category: PolicyCategory;
  one_line_why: string;
  decision_id: string;
  /**
   * Null when the rejected content was never inserted (illegal
   * comment block path) — there's no appeal because there is no
   * row for staff to act on. Renderers should suppress the appeal
   * link and tell the user the block is final.
   */
  appeal_url: string | null;
}

export async function writeModerationNotification(params: {
  recipientId: string;
  targetType: ModerationKind;
  targetId: string | null;
  targetTitle: string | null;
  category: PolicyCategory;
  oneLineWhy: string;
  decisionId: string;
}): Promise<void> {
  // Illegal-comment blocks (and any future "no row to point at"
  // path) get null targetId → null appeal_url. Anything else gets
  // the standard appeal page link.
  const appealUrl =
    params.targetId === null ? null : `/appeal/${params.decisionId}`;
  const payload: ModerationNotificationPayload = {
    target: {
      type: params.targetType,
      id: params.targetId,
      title: params.targetTitle,
    },
    category: params.category,
    one_line_why: params.oneLineWhy,
    decision_id: params.decisionId,
    appeal_url: appealUrl,
  };

  await db.insert(notifications).values({
    userId: params.recipientId,
    kind: "moderation",
    payload,
  });
}
