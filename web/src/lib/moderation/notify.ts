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
  appeal_url: string;
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
  const payload: ModerationNotificationPayload = {
    target: {
      type: params.targetType,
      id: params.targetId,
      title: params.targetTitle,
    },
    category: params.category,
    one_line_why: params.oneLineWhy,
    decision_id: params.decisionId,
    appeal_url: `/appeal/${params.decisionId}`,
  };

  await db.insert(notifications).values({
    userId: params.recipientId,
    kind: "moderation",
    payload,
  });
}
