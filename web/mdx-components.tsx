import type { MDXComponents } from "mdx/types";

// Default MDX components — keeps tags styled by the docs.css rules.
// Override here if a tag needs custom rendering (e.g. anchored headings).
export function useMDXComponents(components: MDXComponents): MDXComponents {
  return { ...components };
}
