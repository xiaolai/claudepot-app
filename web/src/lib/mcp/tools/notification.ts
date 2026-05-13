/**
 * MCP tools — notification inbox (list + mark-read).
 */

import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";

import {
  listNotificationsForUser,
  listNotificationsInputSchema,
  markNotificationsReadForUser,
  markReadInputSchema,
  NOTIFICATION_KINDS,
} from "@/lib/notifications";
import { chargeForTool, checkAuthForTool } from "../policy";
import { formatZodIssues, textResult } from "./helpers";

export function registerNotificationTools(server: McpServer): void {
  /* ── list_notifications ────────────────────────────────────── */
  server.registerTool(
    "list_notifications",
    {
      title: "List your notifications",
      description:
        "Returns the calling user's notifications. Initial fetches come " +
        "newest first; `since` polls are chronological so callers can " +
        "drain overflow windows safely. Requires the notification:read scope.",
      inputSchema: {
        unreadOnly: z.boolean().optional(),
        since: z.iso.datetime().optional(),
        limit: z.number().int().min(1).max(200).optional(),
        kinds: z
          .array(z.enum(NOTIFICATION_KINDS))
          .max(NOTIFICATION_KINDS.length)
          .optional(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("list_notifications", extra);
      if (!a.ok) return a.result;

      const parsed = listNotificationsInputSchema.safeParse(args);
      if (!parsed.success) {
        return textResult(
          `Validation failed: ${formatZodIssues(parsed.error)}`,
          true,
        );
      }

      const c = await chargeForTool("list_notifications", a.ctx.tokenId);
      if (!c.ok) return c.result;

      const result = await listNotificationsForUser(a.ctx.userId, parsed.data);
      return textResult(JSON.stringify(result, null, 2));
    },
  );

  /* ── mark_notifications_read ──────────────────────────────── */
  server.registerTool(
    "mark_notifications_read",
    {
      title: "Mark notifications as read",
      description:
        "Marks notifications as read for the calling user. Pass " +
        "`ids` to mark specific items, or `all: true` to mark every " +
        "unread item. Idempotent. Requires the notification:read scope.",
      inputSchema: {
        ids: z.array(z.uuid()).max(500).optional(),
        all: z.boolean().optional(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("mark_notifications_read", extra);
      if (!a.ok) return a.result;

      const parsed = markReadInputSchema.safeParse(args);
      if (!parsed.success) {
        return textResult(
          `Validation failed: ${formatZodIssues(parsed.error)}`,
          true,
        );
      }

      const c = await chargeForTool("mark_notifications_read", a.ctx.tokenId);
      if (!c.ok) return c.result;

      const result = await markNotificationsReadForUser(a.ctx.userId, parsed.data);
      return textResult(`Marked ${result.updated} notification(s) as read.`);
    },
  );
}
