import type { ReactNode } from "react";
import type { TFunction } from "i18next";
import { Trans, useTranslation } from "react-i18next";
import { EmptyState } from "../../components/primitives/EmptyState";
import { SectionLabel } from "../../components/primitives/SectionLabel";
import { SkeletonRows } from "../../components/primitives/Skeleton";
import { Tag } from "../../components/primitives/Tag";
import { CopyButton } from "../../components/CopyButton";
import type {
  RelayClientServer,
  RelayClientUpstream,
  RelayProbeError,
} from "../../api";

/**
 * Paired relay servers (`relay-servers.json`), read-only, beside the
 * routes. Relay servers are deliberately not routes: this file is the
 * client's only record of them, and pairing, acceptance and launching
 * stay in the CLI (`claudepot relay pair | accept | run`). No control
 * here acts on a server, so the section adds no primary action.
 *
 * Every reason arrives as a fixed code — relay-supplied text never
 * crosses IPC (`commands/relay.rs::relay_client_servers`).
 *
 * The base URL is the only path-like value and has no other copy site,
 * so it carries `title` and a `CopyButton` (path-display state C).
 */
export function RelayServersList({
  servers,
  error,
}: {
  /** `null` while the first probe is in flight. */
  servers: RelayClientServer[] | null;
  /** Raw backend message; composed into a sentence at render time. */
  error: string | null;
}) {
  const { t } = useTranslation("providers");
  return (
    <section aria-label={t("relayServers.label")}>
      <SectionLabel>{t("relayServers.label")}</SectionLabel>
      {error !== null ? (
        <div
          role="alert"
          style={{ fontSize: "var(--fs-sm)", color: "var(--fg)" }}
        >
          {t("relayServers.loadFailed", { message: error })}
        </div>
      ) : servers === null ? (
        <SkeletonRows rows={1} label={t("relayServers.loading")} />
      ) : servers.length === 0 ? (
        <EmptyState
          variant="inline"
          body={
            <>
              {t("relayServers.empty")}{" "}
              <code>claudepot relay pair &lt;host&gt; --code &lt;code&gt;</code>
            </>
          }
          action={null}
        />
      ) : (
        <>
          <p
            style={{
              margin: "0 0 var(--sp-12)",
              fontSize: "var(--fs-sm)",
              color: "var(--fg-muted)",
            }}
          >
            <Trans
              ns="providers"
              i18nKey="relayServers.runNote"
              components={{ code: <code /> }}
            />
          </p>
          <ul
            style={{
              listStyle: "none",
              margin: 0,
              padding: 0,
              display: "flex",
              flexDirection: "column",
              gap: "var(--sp-12)",
            }}
          >
            {servers.map((s) => (
              <ServerRow key={s.serverId} server={s} />
            ))}
          </ul>
        </>
      )}
    </section>
  );
}

function ServerRow({ server }: { server: RelayClientServer }) {
  const { t } = useTranslation("providers");
  return (
    <li
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-8)",
        padding: "var(--sp-12) var(--sp-16)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-3)",
        background: "var(--bg-raised)",
        fontSize: "var(--fs-sm)",
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-8)",
          flexWrap: "wrap",
          minWidth: 0,
        }}
      >
        <h3
          style={{
            margin: 0,
            fontSize: "var(--fs-md)",
            fontWeight: 600,
            color: "var(--fg)",
          }}
        >
          {server.name}
        </h3>
        {server.reachable ? (
          <Tag tone="ok">
            {server.rttMs !== null
              ? t("relayServers.reachableRtt", { ms: server.rttMs })
              : t("relayServers.reachable")}
          </Tag>
        ) : (
          <Tag tone="danger">{t("relayServers.unreachable")}</Tag>
        )}
        <span
          className="mono selectable"
          title={server.baseUrl}
          style={{
            minWidth: 0,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
            color: "var(--fg-muted)",
          }}
        >
          {server.baseUrl}
        </span>
        <CopyButton text={server.baseUrl} />
      </div>
      {!server.reachable && server.error !== null && (
        <span style={{ color: "var(--fg-muted)" }}>
          {probeErrorText(server.error, t)}
        </span>
      )}
      {server.upstreams.length > 0 && (
        <ul
          aria-label={t("relayServers.upstreamsLabel", { name: server.name })}
          style={{
            listStyle: "none",
            margin: 0,
            padding: 0,
            display: "flex",
            flexDirection: "column",
            gap: "var(--sp-4)",
          }}
        >
          {server.upstreams.map((u) => (
            <UpstreamRow
              key={u.id}
              upstream={u}
              isDefault={u.id === server.defaultUpstream}
            />
          ))}
        </ul>
      )}
    </li>
  );
}

function UpstreamRow({
  upstream,
  isDefault,
}: {
  upstream: RelayClientUpstream;
  isDefault: boolean;
}) {
  const { t } = useTranslation("providers");
  return (
    <li
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-8)",
        flexWrap: "wrap",
      }}
    >
      <code style={{ color: "var(--fg)" }}>{upstream.id}</code>
      {isDefault && <Tag tone="accent">{t("relayServers.default")}</Tag>}
      {upstream.eligible ? (
        <Tag tone="ok">{t("relayServers.ready")}</Tag>
      ) : (
        <Tag tone="warn">{t("relayServers.notReady")}</Tag>
      )}
      <span style={{ color: "var(--fg-muted)" }}>
        {upstream.eligible ? readyText(upstream, t) : reasonText(upstream, t)}
      </span>
    </li>
  );
}

type T = TFunction<"providers">;

function readyText(u: RelayClientUpstream, t: T): string {
  const score = u.scoreMs ?? 0;
  if (u.tier === "measured" && u.tierModel !== null && u.ttfbMs !== null) {
    return t("relayServers.tierMeasured", {
      model: u.tierModel,
      ttfb: u.ttfbMs,
      score,
    });
  }
  return t("relayServers.tierConnect", { score });
}

function reasonText(u: RelayClientUpstream, t: T): ReactNode {
  switch (u.reason) {
    case "fingerprint_changed":
    case "not_accepted":
      return (
        <Trans
          ns="providers"
          i18nKey={`relayServers.reason.${u.reason}`}
          components={{ code: <code /> }}
        />
      );
    case "cooldown":
      return t("relayServers.reason.cooldown", { secs: u.cooldownSecs ?? 0 });
    case "saturated": {
      const pct = Math.round(u.saturatedUtilization ?? 0);
      return u.saturatedWindow === "7-day"
        ? t("relayServers.reason.saturated7d", { pct })
        : t("relayServers.reason.saturated5h", { pct });
    }
    case null:
      return t("relayServers.reason.unknown");
    default:
      return t(`relayServers.reason.${u.reason}`);
  }
}

function probeErrorText(code: RelayProbeError, t: T): string {
  return t(`relayServers.probe.${code}`);
}
