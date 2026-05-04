/**
 * Username = identity. Auto-assigned on first OAuth signup from the
 * provider's display-name (or email local-part as a fallback), with a
 * deterministic collision-resolution generator. The user can rename
 * themselves within a grace window after signup; thereafter only an
 * admin can change it.
 *
 * Pattern: dashes, not underscores. ClauDepot's audience comes from
 * GitHub, where dashes are the canonical separator (`vercel-app`,
 * not `vercel_app`). Matches the reserved seeded usernames
 * (`ada`, `claudepot`, `lixiaolai` all pass) and our existing route
 * convention.
 *
 * Adapted from xiaolai's lixiaolai.com forum-handle library.
 */

/* ─── Pattern + sizing ──────────────────────────────────────────── */

const MIN_LEN = 3;
const MAX_LEN = 24;

/**
 * `^[a-z0-9](-?[a-z0-9])*$` — must start and end with letter/digit,
 * no consecutive or trailing dashes. 3-24 chars enforced separately
 * via `.length` so the regex isn't responsible for the size check.
 */
export const USERNAME_REGEX = /^[a-z0-9](?:-?[a-z0-9])*$/;

/* ─── Rate-limit constants for in-grace rename ───────────────────── */

export const SELF_RENAME_GRACE_DAYS = 7;
export const SELF_RENAME_COOLDOWN_MINUTES = 15;
export const MAX_SELF_RENAMES = 3;

/* ─── Reserved set ──────────────────────────────────────────────── */

/**
 * Reserved usernames. Combines:
 *   - Every top-level URL slug under (reader) so a user can't
 *     shadow a real route.
 *   - System / role identities that look-like-a-user but aren't.
 *   - the seeded fixture usernames (so a fresh OAuth user can't
 *     claim a name another seeded row already owns at the DB level —
 *     redundant with the unique index, but produces a cleaner error
 *     than a constraint violation).
 */
export const RESERVED_USERNAMES: ReadonlySet<string> = new Set([
  // Top-level routes (must match src/app/(reader) directory listing)
  "about",
  "admin",
  "api",
  "auth",
  "briefs",
  "c",
  "login",
  "mod",
  "new",
  "notifications",
  "post",
  "projects",
  "saved",
  "search",
  "settings",
  "submit",
  "top",
  "u",
  // Static + meta paths
  "favicon",
  "icon",
  "robots",
  "sitemap",
  // Identity / role ambiguity
  "administrator",
  "moderator",
  "root",
  "system",
  "staff",
  "support",
  "help",
  "everyone",
  "here",
  "channel",
  "user",
  "users",
  "null",
  "undefined",
  "www",
  "mail",
  // Site-owner brand identities — locked so no one can squat them.
  // xiaolai is the current owner of the @xiaolai row post-rename;
  // lixiaolai is reserved against re-claim from drive-by signups.
  "xiaolai",
  "lixiaolai",
  "claudepot",
]);

/* ─── Public surface ────────────────────────────────────────────── */

export function normalizeUsername(input: string): string {
  return input.trim().toLowerCase().replace(/^@+/, "");
}

export function isValidUsernameShape(input: string): boolean {
  if (input.length < MIN_LEN || input.length > MAX_LEN) return false;
  return USERNAME_REGEX.test(input);
}

export function isReservedUsername(input: string): boolean {
  return RESERVED_USERNAMES.has(normalizeUsername(input));
}

/* ─── Seed derivation ───────────────────────────────────────────── */

function randomHex(bytes: number): string {
  const arr = new Uint8Array(bytes);
  globalThis.crypto.getRandomValues(arr);
  let out = "";
  for (const byte of arr) out += byte.toString(16).padStart(2, "0");
  return out;
}

function fallbackRandom(): string {
  return `user-${randomHex(3)}`;
}

/**
 * Replace non-[a-z0-9] (after lowercasing) with a single dash; collapse
 * consecutive dashes; trim leading/trailing dashes. Pure transform —
 * no length, regex-shape, or reservation check.
 */
function sanitize(raw: string): string {
  return raw
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[^a-z0-9-]+/g, "-")
    .replace(/-+/g, "-")
    .replace(/^-+|-+$/g, "");
}

function prefixIfShort(value: string): string {
  return value.length >= MIN_LEN ? value : `${value}-${randomHex(2)}`;
}

/**
 * Derive a username seed from an OAuth display name (or any free-text
 * label). The result is guaranteed to satisfy the regex and the size
 * range, but is NOT guaranteed to be unique in the DB or non-reserved
 * — the caller must pass the seed through `generateUsernameCandidates`
 * and check each candidate against availability.
 */
export function usernameFromName(name: string | null | undefined): string {
  const raw = (name ?? "").trim();
  if (!raw) return fallbackRandom();
  const sanitized = sanitize(raw);
  if (!sanitized) return fallbackRandom();
  const truncated = sanitized.slice(0, MAX_LEN).replace(/-+$/, "");
  const sized = prefixIfShort(truncated || "user");
  return isValidUsernameShape(sized) ? sized : fallbackRandom();
}

/** Derive a username seed from an email address (local-part, +tag stripped). */
export function usernameFromEmail(email: string): string {
  const local = (email.split("@")[0] ?? "").split("+")[0] ?? "";
  return usernameFromName(local);
}

/* ─── Candidate generator ───────────────────────────────────────── */

/**
 * Lazy sequence of candidate usernames starting with `seed`:
 *
 *   seed, seed-2, seed-3, ..., seed-99, seed-<rand4>, seed-<rand4>, ...
 *
 * The caller pulls candidates and checks each against the DB and the
 * reserved set. The generator truncates `seed` so `seed-<suffix>` never
 * exceeds `MAX_LEN`. Random-suffix candidates are deduped within the
 * generator so the caller never sees the same candidate twice.
 */
export function* generateUsernameCandidates(seed: string): Generator<string> {
  const base = seed.length > MAX_LEN ? seed.slice(0, MAX_LEN) : seed;
  const yielded = new Set<string>();

  function* emit(candidate: string): Generator<string> {
    if (!isValidUsernameShape(candidate)) return;
    if (yielded.has(candidate)) return;
    yielded.add(candidate);
    yield candidate;
  }

  yield* emit(base);

  for (let i = 2; i <= 99; i += 1) {
    const suffix = `-${i}`;
    const room = MAX_LEN - suffix.length;
    const trimmed = (base.length > room ? base.slice(0, room) : base).replace(
      /-+$/,
      "",
    );
    if (!trimmed) continue;
    yield* emit(`${trimmed}${suffix}`);
  }

  // Random-suffix tail — caller decides when to stop pulling.
  while (true) {
    const suffix = `-${randomHex(2)}`;
    const room = MAX_LEN - suffix.length;
    const trimmed = (base.length > room ? base.slice(0, room) : base).replace(
      /-+$/,
      "",
    );
    if (!trimmed) {
      // base became empty — fall back to a fully random candidate.
      yield* emit(`user-${randomHex(3)}`);
      continue;
    }
    yield* emit(`${trimmed}${suffix}`);
  }
}

/* ─── Self-rename eligibility ───────────────────────────────────── */

export type SelfRenameState = {
  createdAt: Date;
  selfUsernameRenameCount: number;
  usernameLastChangedAt: Date | null;
};

export type SelfRenameDecision =
  | { ok: true }
  | { ok: false; reason: "grace_expired" | "count_exceeded" | "cooldown" };

/**
 * Decide whether the user can rename themselves right now. The grace
 * window starts at signup; once it ends or the user has burned all
 * their renames, only an admin can change the username (admin path
 * not implemented here — callers enforce by role check).
 */
export function canSelfRename(
  state: SelfRenameState,
  now: Date = new Date(),
): SelfRenameDecision {
  const graceDeadline = new Date(
    state.createdAt.getTime() + SELF_RENAME_GRACE_DAYS * 24 * 60 * 60 * 1000,
  );
  if (now >= graceDeadline) return { ok: false, reason: "grace_expired" };

  if (state.selfUsernameRenameCount >= MAX_SELF_RENAMES) {
    return { ok: false, reason: "count_exceeded" };
  }

  if (state.usernameLastChangedAt) {
    const cooldownUntil = new Date(
      state.usernameLastChangedAt.getTime() +
        SELF_RENAME_COOLDOWN_MINUTES * 60 * 1000,
    );
    if (now < cooldownUntil) return { ok: false, reason: "cooldown" };
  }

  return { ok: true };
}
