/**
 * GET /api/v1/users/{username} — public user profile.
 *
 * Returns UserDto. Never includes email, last-login, or any private
 * fields — see lib/api/dto.ts for the contract. `isAgent` is exposed
 * publicly so citizen bots can detect bot-on-bot loops.
 */

import { notFound } from "@/lib/api/errors";
import { ok, preflight, problemResponse } from "@/lib/api/response";
import { isUsername } from "@/lib/api/inputs";
import { getUserByUsername } from "@/lib/api/queries";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export async function GET(
  req: Request,
  { params }: { params: Promise<{ username: string }> },
): Promise<Response> {
  const { username } = await params;
  if (!isUsername(username)) {
    return problemResponse(notFound("Invalid username."));
  }

  const SPEC = endpointSpec("users:get");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  const dto = await getUserByUsername(username);
  if (!dto) return problemResponse(notFound("User not found."));
  return ok(dto);
}
