// Global → Tips. CC tip ledger root panel.
//
// Loads the catalog (extracted from the user's CC binary), joins
// it with `~/.claude.json::tipsHistory` and Claudepot's snapshot
// log, and renders a searchable, filterable list. See
// `dev-docs/cc-tips-ledger.md` for the design.

import { Trans, useTranslation } from "react-i18next";
import { Button } from "../../components/primitives/Button";
import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";
import { TipsList } from "./tips/TipsList";
import { useTipsCatalog } from "./tips/useTipsCatalog";

export function TipsPanel() {
  const { t } = useTranslation("global");
  const { data, loading, error, refresh, refreshing } = useTipsCatalog();

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        minHeight: 0,
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "baseline",
          gap: "var(--sp-10)",
          padding: "var(--sp-12) var(--sp-14) var(--sp-6)",
          flexWrap: "wrap",
        }}
      >
        <h2
          style={{
            fontSize: "var(--fs-sm)",
            fontWeight: 500,
            color: "var(--fg)",
            margin: 0,
          }}
        >
          {t("tips.title")}
        </h2>
        <span
          style={{
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-faint)",
            flex: 1,
          }}
        >
          {data ? (
            <Trans
              ns="global"
              i18nKey="tips.headerSummary"
              components={{ strong: <strong /> }}
              values={{
                extracted: data.extracted_count,
                known: data.known_count,
                version: data.catalog_version,
                partial: data.partial ? t("tips.partialSuffix") : "",
                startup: data.current_num_startups,
              }}
            />
          ) : (
            t("tips.loadingCatalog")
          )}
        </span>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => void refresh()}
          disabled={refreshing}
          glyph={NF.refresh}
        >
          {refreshing ? t("tips.refreshing") : t("tips.refresh")}
        </Button>
      </div>
      <p
        style={{
          padding: "0 var(--sp-14) var(--sp-8)",
          margin: 0,
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-faint)",
        }}
      >
        <Trans
          ns="global"
          i18nKey="tips.privacyNote"
          components={{ code: <code /> }}
        />
      </p>
      {loading && (
        <div
          style={{
            padding: "var(--sp-16)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg-faint)",
            textAlign: "center",
          }}
        >
          {t("tips.readingBinary")}
        </div>
      )}
      {error && (
        <div
          style={{
            padding: "var(--sp-12) var(--sp-14)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg-muted)",
            background: "var(--bg-sunken)",
            border: "var(--bw-hair) solid var(--line-strong)",
            margin: "var(--sp-8) var(--sp-14)",
            borderRadius: "var(--r-2)",
            display: "flex",
            gap: "var(--sp-8)",
            alignItems: "flex-start",
          }}
        >
          <Glyph g={NF.warn} />
          <div>
            <strong>{t("tips.unavailable")}</strong>
            <div style={{ marginTop: "var(--sp-4)", color: "var(--fg-faint)" }}>
              {error}
            </div>
            <div
              style={{
                marginTop: "var(--sp-6)",
                fontSize: "var(--fs-2xs)",
                color: "var(--fg-faint)",
              }}
            >
              {t("tips.installHint")}
            </div>
          </div>
        </div>
      )}
      {data && !loading && !error && (
        <TipsList tips={data.tips} counts={data.counts} />
      )}
    </div>
  );
}
