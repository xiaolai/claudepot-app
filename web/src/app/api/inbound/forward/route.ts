/**
 * Resend Inbound forwarder — replaces the CF Email Routing forwarder
 * that was dismantled on 2026-05-19 when we cut the apex MX over to
 * Resend's inbound SMTP (inbound-smtp.us-east-1.amazonaws.com).
 *
 * Architecture: Resend receives every mail to *@claudepot.com, fires
 * an `email.received` Svix-signed webhook to this endpoint, and this
 * handler asks Resend to re-send the message to FORWARD_TO. The body
 * never crosses our process (Resend's forward helper fetches the raw
 * email server-side), so attachments, headers, and HTML are preserved.
 *
 * Security model:
 *   - Svix signature verification gates every call. Resend signs with
 *     RESEND_WEBHOOK_SECRET (set in the Resend dashboard when the
 *     webhook is created, mirrored into Vercel env).
 *   - FORWARD_TO is hardcoded. An attacker who somehow forged a valid
 *     signature still can't relay to an arbitrary destination — the
 *     forward target is compiled in.
 *   - Returns 401 on signature failure so Resend retries with the same
 *     event id; returns 200 on success or "skipped" events so Resend
 *     drops them from the retry queue.
 *
 * Operational notes:
 *   - The webhook URL to register in the Resend dashboard:
 *       https://claudepot.com/api/inbound/forward
 *   - Event subscription: `email.received` (only).
 *   - Required env: RESEND_API_KEY, RESEND_WEBHOOK_SECRET.
 *   - If RESEND_WEBHOOK_SECRET is missing the handler fails closed
 *     (500); Resend will keep retrying, which is the right behavior
 *     during config drift.
 */

import { NextResponse } from "next/server";
import { Resend } from "resend";
import { Webhook } from "svix";

const FORWARD_TO = "xiaolaidev+claudepot@gmail.com";
const FORWARD_FROM = "ClauDepot Inbound <forward@claudepot.com>";

type ResendInboundEvent = {
  type: string;
  data?: {
    email_id?: string;
    to?: string | string[];
    from?: string;
    subject?: string;
  };
};

export async function POST(req: Request) {
  const secret = process.env.RESEND_WEBHOOK_SECRET;
  const apiKey = process.env.RESEND_API_KEY;
  if (!secret) {
    return NextResponse.json(
      { error: "RESEND_WEBHOOK_SECRET not configured" },
      { status: 500 },
    );
  }
  if (!apiKey) {
    return NextResponse.json(
      { error: "RESEND_API_KEY not configured" },
      { status: 500 },
    );
  }

  // Svix verifies against the *raw* request body. Read it as text and
  // pass to Webhook.verify; don't await req.json() first, that would
  // re-stringify and break the signature.
  const rawBody = await req.text();
  const svixHeaders = {
    "svix-id": req.headers.get("svix-id") ?? "",
    "svix-timestamp": req.headers.get("svix-timestamp") ?? "",
    "svix-signature": req.headers.get("svix-signature") ?? "",
  };

  let event: ResendInboundEvent;
  try {
    event = new Webhook(secret).verify(
      rawBody,
      svixHeaders,
    ) as ResendInboundEvent;
  } catch {
    return NextResponse.json(
      { error: "invalid signature" },
      { status: 401 },
    );
  }

  // Only act on inbound mail. Other event types (e.g. email.sent for
  // the forwarded copy itself, email.delivered for the original send,
  // etc.) are acked with 200 so Resend stops retrying.
  if (event.type !== "email.received") {
    return NextResponse.json(
      { skipped: true, reason: `unhandled event ${event.type}` },
      { status: 200 },
    );
  }

  const emailId = event.data?.email_id;
  if (!emailId) {
    return NextResponse.json(
      { error: "missing email_id in event payload" },
      { status: 400 },
    );
  }

  const resend = new Resend(apiKey);
  await resend.emails.receiving.forward({
    emailId,
    to: FORWARD_TO,
    from: FORWARD_FROM,
  });

  return NextResponse.json({ forwarded: true, to: FORWARD_TO, emailId });
}
