// Grouping and rendering of one result line. See CATEGORY_LABELS for why
// categories are a heading rather than a filter.

import type { EnvCategory, EnvVarState } from "../../../types/ccEnv";
import { EnvVarRow } from "./EnvVarRow";
import type { ControlHandlers } from "./EnvVarControl";

/** One rendered line: a category heading, or a variable. */
export type Item =
  | { kind: "header"; category: string; label: string; count: number }
  | { kind: "row"; state: EnvVarState };

/**
 * Group filtered results by category, in the generator's order.
 *
 * `categories` comes from the generated spec rather than a table mirrored
 * here: a hand copy drifts the first time the generator adds or reorders one,
 * and the drift is invisible — rows just quietly move to the bottom.
 */
export function groupByCategory(
  rows: EnvVarState[],
  categories: EnvCategory[],
): Item[] {
  const buckets = new Map<string, EnvVarState[]>();
  for (const r of rows) {
    const list = buckets.get(r.spec.category) ?? [];
    list.push(r);
    buckets.set(r.spec.category, list);
  }
  const out: Item[] = [];
  for (const { key, label } of categories) {
    const list = buckets.get(key);
    if (!list || list.length === 0) continue;
    out.push({ kind: "header", category: key, label, count: list.length });
    for (const state of list) out.push({ kind: "row", state });
  }
  // A category the spec did not describe still renders, under its raw key.
  // Silently dropping rows would be far worse than an ugly heading, and this
  // is the failure mode if the two ever do get out of step.
  for (const [category, list] of buckets) {
    if (categories.some((c) => c.key === category)) continue;
    out.push({ kind: "header", category, label: category, count: list.length });
    for (const state of list) out.push({ kind: "row", state });
  }
  return out;
}

/** Stable React key. Headers and rows share one list, so BOTH sides are
 *  namespaced — prefixing only the headers left `cat:misc` reachable as a
 *  variable name, and React would then reuse a heading's element for a row. */
export function itemKey(item: Item): string {
  return item.kind === "header"
    ? `cat:${item.category}`
    : `env:${item.state.spec.name}`;
}

export function ItemView({
  item,
  busy,
  handlers,
}: {
  item: Item;
  busy: boolean;
  handlers: ControlHandlers;
}) {
  if (item.kind === "header") {
    return (
      <h4 className="envvar-group">
        {item.label} <span>{item.count}</span>
      </h4>
    );
  }
  return <EnvVarRow state={item.state} busy={busy} handlers={handlers} />;
}

