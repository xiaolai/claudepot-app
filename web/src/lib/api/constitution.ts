/**
 * Constitution payload — public editorial sources surfaced to citizen
 * bots. Reads the same files the /office/* routes already render
 * (editorial/audience.md, transparency.md, rubric.yml), assembles them
 * into a single ConstitutionDto, and computes a stable version + ETag.
 *
 * The /office/voice URL is a SLUG, not a filename — the page reads
 * editorial/audience.md. The `audience` field in this DTO is the same
 * file. Citizens that need "the voice doc" should read `audience`.
 *
 * Versioning:
 *   - Vercel deploys: `VERCEL_GIT_COMMIT_SHA` is the truth-source.
 *   - Local `next dev` and non-Vercel deploys: fall back to a
 *     content hash over the four payload pieces, so the version
 *     changes whenever the editorial sources change without depending
 *     on a clean git tree.
 *
 * The version doubles as the ETag value (without quotes; the route
 * adds those per RFC 7232).
 *
 * Memoization: the four files are bundled into the deployment and
 * never change during the lifetime of a serverless instance, so we
 * read once and cache. In local dev, restart `next dev` to pick up
 * editorial changes — same as every other static editorial reader.
 */

import { createHash } from "node:crypto";

import {
  readAudienceMd,
  readPublicRubricView,
  readTransparencyMd,
} from "@/lib/editorial-spec";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import type { ConstitutionDto } from "./dto";

const AUDIENCE_PATH = "editorial/audience.md";
const TRANSPARENCY_PATH = "editorial/transparency.md";
const RUBRIC_PATH = "editorial/rubric.yml";

function readRubricYaml(): string {
  return readFileSync(resolve(process.cwd(), RUBRIC_PATH), "utf-8");
}

/**
 * Build the version string. Prefer Vercel's deploy SHA; fall back to
 * a sha256 of the content (truncated to 12 chars — enough to make
 * accidental collisions astronomical without bloating the ETag).
 */
function computeVersion(
  audienceMd: string,
  transparencyMd: string,
  rubricYaml: string,
): string {
  const sha = process.env.VERCEL_GIT_COMMIT_SHA;
  if (sha && sha.length > 0) return sha;

  const hash = createHash("sha256");
  hash.update(audienceMd);
  hash.update("\0");
  hash.update(transparencyMd);
  hash.update("\0");
  hash.update(rubricYaml);
  return hash.digest("hex").slice(0, 12);
}

let cached: ConstitutionDto | null = null;

export function getConstitution(): ConstitutionDto {
  if (cached) return cached;

  const audienceMd = readAudienceMd();
  const transparencyMd = readTransparencyMd();
  const rubricYaml = readRubricYaml();
  const rubricPublic = readPublicRubricView();
  const version = computeVersion(audienceMd, transparencyMd, rubricYaml);

  cached = {
    version,
    generatedAt: new Date().toISOString(),
    audience: { path: AUDIENCE_PATH, markdown: audienceMd },
    rubric: { path: RUBRIC_PATH, yaml: rubricYaml, public: rubricPublic },
    transparency: { path: TRANSPARENCY_PATH, markdown: transparencyMd },
  };
  return cached;
}

/**
 * RFC 7232 strong ETag — the version wrapped in double quotes.
 * Matches what the route emits via the `ETag:` response header.
 */
export function etagFor(version: string): string {
  return `"${version}"`;
}

/**
 * If-None-Match comparison. A client may send a single tag, a list,
 * or `*`. We match on exact string equality after stripping a single
 * pair of surrounding quotes — lenient enough to absorb middlebox
 * quirks without being permissive about the W/ weak prefix (we issue
 * strong ETags only, so a W/ revalidation should miss).
 */
export function ifNoneMatchMatches(header: string | null, etag: string): boolean {
  if (!header) return false;
  // Wildcard revalidation — RFC 7232 §3.2: matches any current entity.
  if (header.trim() === "*") return true;
  return header
    .split(",")
    .map((s) => s.trim())
    .some((tag) => tag === etag);
}

/**
 * Test seam — the cache holds a global by design (one constitution per
 * process), but tests need to force a re-read between assertions.
 */
export function _resetConstitutionCacheForTests(): void {
  cached = null;
}
