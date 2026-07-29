// Search field + the two filter facets above the results list.
//
// Split out of `EnvVarsPane` to keep that file under the 350-LOC shard
// limit (rules/design.md). It is a pure presentational component: every
// piece of state it renders lives in `useEnvVarFilter`, which the pane
// owns.

import { useId } from "react";
import { FilterChip, IconButton, Input } from "../../../components/primitives";
import { NF } from "../../../icons";
import type { EnvControl } from "../../../types/ccEnv";
import {
  CONTROLS,
  SAFETY_FILTERS,
  type SafetyFilter,
} from "./useEnvVarFilter";

export function EnvVarsToolbar({
  query,
  onQueryChange,
  controls,
  safety,
  onToggleControl,
  onToggleSafety,
  showInfo,
  onToggleInfo,
  onReload,
}: {
  query: string;
  onQueryChange: (next: string) => void;
  controls: Set<EnvControl>;
  safety: Set<SafetyFilter>;
  onToggleControl: (c: EnvControl) => void;
  onToggleSafety: (f: SafetyFilter) => void;
  showInfo: boolean;
  onToggleInfo: () => void;
  onReload: () => void;
}) {
  const typeFacetId = useId();
  const attrFacetId = useId();

  return (
    <>
      <div className="envvar-toolbar">
        <Input
          glyph={NF.search}
          value={query}
          placeholder="Search name or description…"
          aria-label="Search environment variables"
          onChange={(e) => onQueryChange(e.target.value)}
          style={{ flex: 1 }}
        />
        <IconButton
          glyph={NF.info}
          title="About environment variables"
          aria-label="About environment variables"
          aria-expanded={showInfo}
          onClick={onToggleInfo}
        />
        <IconButton
          glyph={NF.refresh}
          title="Reload from disk"
          aria-label="Reload from disk"
          onClick={onReload}
        />
      </div>

      {/* Two facets, labelled, because they do not combine the same way and
          used to be one undifferentiated row of eight identical chips.
          Picking two types WIDENS the results (a variable is exactly one
          type, so the group is an OR); picking two attributes NARROWS them
          (a variable can be several at once, so the group is an AND). Same
          affordance, opposite effect — the labels carry the rule rather than
          leaving the user to infer it from a changing result count. */}
      <div className="envvar-facets">
        <div className="envvar-facet">
          <span className="envvar-facet-label" id={typeFacetId}>
            Type <span className="envvar-facet-rule">— any</span>
          </span>
          <div
            className="envvar-chips"
            role="group"
            aria-labelledby={typeFacetId}
          >
            {CONTROLS.map((c) => (
              <FilterChip
                key={c}
                active={controls.has(c)}
                onToggle={() => onToggleControl(c)}
              >
                {c}
              </FilterChip>
            ))}
          </div>
        </div>
        <div className="envvar-facet">
          <span className="envvar-facet-label" id={attrFacetId}>
            Attributes <span className="envvar-facet-rule">— all</span>
          </span>
          <div
            className="envvar-chips"
            role="group"
            aria-labelledby={attrFacetId}
          >
            {SAFETY_FILTERS.map((f) => (
              <FilterChip
                key={f.key}
                active={safety.has(f.key)}
                onToggle={() => onToggleSafety(f.key)}
              >
                {f.label}
              </FilterChip>
            ))}
          </div>
        </div>
      </div>
    </>
  );
}
