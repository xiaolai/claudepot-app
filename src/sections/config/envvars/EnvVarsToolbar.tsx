// Search field + the two filter facets above the results list.
//
// Split out of `EnvVarsPane` to keep that file under the 350-LOC shard
// limit (rules/design.md). It is a pure presentational component: every
// piece of state it renders lives in `useEnvVarFilter`, which the pane
// owns.

import { useId } from "react";
import { useTranslation } from "react-i18next";
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
  const { t } = useTranslation("config");
  const typeFacetId = useId();
  const attrFacetId = useId();

  return (
    <>
      <div className="envvar-toolbar">
        <Input
          glyph={NF.search}
          value={query}
          placeholder={t("envvars.searchPlaceholder")}
          aria-label={t("envvars.searchAria")}
          onChange={(e) => onQueryChange(e.target.value)}
          style={{ flex: 1 }}
        />
        <IconButton
          glyph={NF.info}
          title={t("envvars.aboutAria")}
          aria-label={t("envvars.aboutAria")}
          aria-expanded={showInfo}
          onClick={onToggleInfo}
        />
        <IconButton
          glyph={NF.refresh}
          title={t("envvars.reload")}
          aria-label={t("envvars.reload")}
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
            {t("envvars.facetType")}{" "}
            <span className="envvar-facet-rule">{t("envvars.facetAny")}</span>
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
            {t("envvars.facetAttributes")}{" "}
            <span className="envvar-facet-rule">{t("envvars.facetAll")}</span>
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
                {t(f.labelKey)}
              </FilterChip>
            ))}
          </div>
        </div>
      </div>
    </>
  );
}
