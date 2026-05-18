import type { CSSProperties } from "react";

/**
 * GitHub Octocat brand mark, inline SVG.
 *
 * `lucide-react` v1+ removed brand icons for trademark reasons (see
 * `.claude/rules/design.md` brand-mark exception). This mark is the
 * sanctioned inline-SVG fallback for the *single* purpose of marking
 * "this project's branch has an open PR on GitHub." It sits in
 * secondary chrome (project rows, header next to a branch), uses
 * `currentColor` so it inherits the theme, and is never a primary
 * navigation element.
 *
 * SVG path is GitHub's published 24×24 octocat (CC-BY 4.0). Kept as
 * a literal here so it ships with the bundle and doesn't require a
 * runtime fetch.
 */

interface Props {
  size?: number;
  className?: string;
  style?: CSSProperties;
  /** Accessible label. Omit for decorative use (aria-hidden). */
  "aria-label"?: string;
  title?: string;
}

export function BrandGithubMark({
  size = 14,
  className,
  style,
  "aria-label": ariaLabel,
  title,
}: Props) {
  const decorative = ariaLabel === undefined;
  return (
    <svg
      role={decorative ? undefined : "img"}
      aria-hidden={decorative}
      aria-label={ariaLabel}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="currentColor"
      className={className}
      style={{ display: "inline-block", verticalAlign: "middle", ...style }}
    >
      {title && <title>{title}</title>}
      <path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.111.82-.261.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12" />
    </svg>
  );
}
