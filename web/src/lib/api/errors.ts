/**
 * RFC 7807 problem+json error shape, plus typed constructors.
 *
 * Every API route returns either { data: ... } on success or one of these
 * Problem objects on failure. The `type` URI is a stable, documentable
 * identifier — clients can switch on it without parsing strings.
 */

export type Problem = {
  type: string;
  title: string;
  status: number;
  detail?: string;
  errors?: Array<{ field?: string; message: string }>;
};

const BASE = "https://claudepot.com/api/errors";

export const PROBLEM_TYPES = {
  unauthorized: `${BASE}/unauthorized`,
  forbidden: `${BASE}/forbidden`,
  rateLimited: `${BASE}/rate-limited`,
  validation: `${BASE}/validation`,
  notFound: `${BASE}/not-found`,
  conflict: `${BASE}/conflict`,
  internal: `${BASE}/internal`,
  serviceUnavailable: `${BASE}/service-unavailable`,
} as const;

export function unauthorized(detail?: string): Problem {
  return {
    type: PROBLEM_TYPES.unauthorized,
    title: "Unauthorized",
    status: 401,
    detail,
  };
}

export function forbidden(detail?: string): Problem {
  return {
    type: PROBLEM_TYPES.forbidden,
    title: "Forbidden",
    status: 403,
    detail,
  };
}

export function rateLimited(detail: string, resetAt: Date): Problem {
  return {
    type: PROBLEM_TYPES.rateLimited,
    title: "Rate limit exceeded",
    status: 429,
    detail: `${detail} Resets at ${resetAt.toISOString()}.`,
  };
}

export function validation(
  detail: string,
  errors?: Array<{ field?: string; message: string }>,
): Problem {
  return {
    type: PROBLEM_TYPES.validation,
    title: "Validation failed",
    status: 422,
    detail,
    errors,
  };
}

export function notFound(detail?: string): Problem {
  return {
    type: PROBLEM_TYPES.notFound,
    title: "Not found",
    status: 404,
    detail,
  };
}

export function conflict(detail?: string): Problem {
  return {
    type: PROBLEM_TYPES.conflict,
    title: "Conflict",
    status: 409,
    detail,
  };
}

export function internal(detail?: string): Problem {
  return {
    type: PROBLEM_TYPES.internal,
    title: "Internal server error",
    status: 500,
    detail,
  };
}

/**
 * Use for transient infra failures (DB unreachable, downstream timeout).
 * Clients should retry with backoff. The detail string is intentionally
 * generic — never include raw exception messages, which can leak hostnames,
 * SQL fragments, or other ops information.
 */
export function serviceUnavailable(detail?: string): Problem {
  return {
    type: PROBLEM_TYPES.serviceUnavailable,
    title: "Service unavailable",
    status: 503,
    detail: detail ?? "A required service is temporarily unavailable. Retry shortly.",
  };
}
