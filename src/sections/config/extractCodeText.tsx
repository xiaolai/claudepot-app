import { isValidElement, type ReactNode } from "react";

/**
 * Flatten a react-markdown `<code>` subtree back to its source text.
 *
 * Needed because a fenced block arrives as a React element tree, and the
 * two things that consume one — the config preview and the transcript
 * viewer — both need the original characters to hand to mermaid.
 *
 * Shared rather than copied: it was private to `MarkdownRenderer` until
 * a second renderer needed it, and two copies of a recursive flattener
 * is how one of them quietly stops handling a node kind the other does.
 */
export function extractCodeText(node: ReactNode): string {
  if (typeof node === "string") return node;
  if (typeof node === "number") return String(node);
  if (node === null || node === undefined || typeof node === "boolean") return "";
  if (Array.isArray(node)) return node.map(extractCodeText).join("");
  if (isValidElement(node)) {
    const props = node.props as { children?: ReactNode };
    return extractCodeText(props.children);
  }
  return "";
}
