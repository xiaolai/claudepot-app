/**
 * Same-origin path resolver used to sanitise OAuth/login `?callbackUrl=`
 * inputs. Returns the input only if it's a safe local path; otherwise
 * returns "/" so we never bounce a victim to an attacker-controlled
 * origin.
 *
 * The regex requires a leading "/" followed by a character that is
 * neither "/" nor "\". This blocks:
 *   - protocol-relative URLs ("//evil.example")
 *   - backslash-bypass paths ("/\evil.example") — modern browsers
 *     normalise "\" to "/" in URL paths, so "/\evil" lands on
 *     "//evil" → external origin. CVE-class issue if missed.
 *   - bare "/" (length 1) — safe fallback already handles this.
 */
const SAFE_PATH = /^\/[^/\\]/;

export function safeCallback(raw: string | string[] | undefined): string {
  const value = Array.isArray(raw) ? raw[0] : raw;
  if (typeof value === "string" && SAFE_PATH.test(value)) {
    return value;
  }
  return "/";
}
