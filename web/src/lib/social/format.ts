import type { Post } from "./types";

const X_LIMIT = 280;
const X_URL_LENGTH = 23;
const BLUESKY_LIMIT = 300;

export interface FormattedPost {
  text: string;
  truncated: boolean;
}

export function formatForX(post: Post): FormattedPost {
  const urlOverhead = post.url ? X_URL_LENGTH + 1 : 0;
  const textBudget = X_LIMIT - urlOverhead;
  const truncated = post.text.length > textBudget;
  const text = truncated ? post.text.slice(0, textBudget - 1) + "…" : post.text;
  return { text: post.url ? `${text} ${post.url}` : text, truncated };
}

export function formatForBluesky(post: Post): FormattedPost {
  const urlOverhead = post.url ? post.url.length + 1 : 0;
  const textBudget = BLUESKY_LIMIT - urlOverhead;
  const truncated = post.text.length > textBudget;
  const text = truncated ? post.text.slice(0, textBudget - 1) + "…" : post.text;
  return { text: post.url ? `${text} ${post.url}` : text, truncated };
}
