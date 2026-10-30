// Settings → Relay: the Accounts and Published sections.
import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";

import {
  relayApi,
  type RelayAccount,
  type RelayPublication,
  type RelayStatus,
  type RelayUpstreamLive,
} from "../../api";
import { Button } from "../../components/primitives/Button";
import { DevBadge } from "../../components/primitives/DevBadge";
import { EmptyState } from "../../components/primitives/EmptyState";
import { SectionLabel } from "../../components/primitives/SectionLabel";
import type { RelayPaneController } from "./useRelayPaneController";
import { FAINT, ROW_META, ROW_NAME, SECTION } from "./RelayPane.styles";

const INELIGIBLE_LABEL = {
  cc_active: "relay.ineligible.cc_active",
  no_credentials: "relay.ineligible.no_credentials",
} as const;

export function RelayAccountsSection({ c, status }: { c: RelayPaneController; status: RelayStatus }) {
  const { t } = useTranslation("settings");
  const locked = status.serving;
  const published = new Set(
    status.publications
      .filter((p) => p.kind === "account")
      .map((p) => p.accountEmail)
      .filter((e): e is string => e !== null),
  );
  return (
    <section style={SECTION}>
      <SectionLabel>{t("relay.accountsHeading")}</SectionLabel>
      <div style={FAINT}>{t("relay.accountsWhy")}</div>
      <div style={FAINT}>{t("relay.copiedLogin")}</div>
      {status.accounts.length === 0 ? (
        // Accounts are added in the Accounts section; this pane has no
        // route there, and the sidebar is one click away.
        <EmptyState variant="inline" body={t("relay.noAccounts")} action={null} />
      ) : (
        <ul className="remote-devices">
          {status.accounts.map((a) => (
            <AccountRow
              key={a.email}
              a={a}
              published={published.has(a.email)}
              disabled={c.busy !== null || locked}
              publishing={c.busy === `publish-${a.email}`}
              onPublish={() =>
                c.run(`publish-${a.email}`, () => relayApi.relayPublish(a.email), t("relay.publishedOk"))
              }
            />
          ))}
        </ul>
      )}
      {/* Every Publish is off while any change is in flight; the one
          running says so on its own button. */}
      {locked ? (
        <div style={FAINT}>{t("relay.lockedReason")}</div>
      ) : (
        c.busy !== null &&
        status.accounts.length > 0 && <div style={FAINT}>{t("relay.busyReason")}</div>
      )}
    </section>
  );
}

export function RelayPublicationsSection({
  c,
  status,
}: {
  c: RelayPaneController;
  status: RelayStatus;
}) {
  const { t } = useTranslation("settings");
  const locked = status.serving;
  const liveById = new Map(status.live.upstreams.map((u) => [u.id, u]));
  return (
    <section style={SECTION}>
      <SectionLabel>{t("relay.publicationsHeading")}</SectionLabel>
      {status.publications.length === 0 ? (
        // The next step — Publish — is on each account row just above.
        <EmptyState variant="inline" body={t("relay.noPublications")} action={null} />
      ) : (
        <ul className="remote-devices">
          {status.publications.map((p) => (
            <li key={p.id}>
              <span style={ROW_NAME}>
                <PublicationName p={p} /> <DevBadge>{p.id}</DevBadge>
              </span>
              <span className="mono-cap" style={ROW_META}>
                <PublicationState status={status} live={liveById.get(p.id)} />
              </span>
              <Button
                variant="ghost"
                disabled={c.busy !== null || locked}
                onClick={() =>
                  c.run(`unpublish-${p.id}`, () => relayApi.relayUnpublish(p.id), t("relay.unpublishedOk"))
                }
              >
                {c.busy === `unpublish-${p.id}` ? t("relay.unpublishing") : t("relay.unpublish")}
              </Button>
            </li>
          ))}
        </ul>
      )}
      {status.publications.length > 0 &&
        (locked ? (
          <div style={FAINT}>{t("relay.lockedReason")}</div>
        ) : (
          c.busy !== null && <div style={FAINT}>{t("relay.busyReason")}</div>
        ))}
    </section>
  );
}

/** What the publication serves: an account by email, or a route by
 *  name. Never its id, which is a slug for the DevBadge. */
export function publicationLabel(p: RelayPublication, t: TFunction<"settings">): string {
  if (p.kind === "route") {
    return p.routeName ? t("relay.publicationRoute", { name: p.routeName }) : t("relay.routeGone");
  }
  return p.accountEmail ?? t("relay.accountGone");
}

function PublicationName({ p }: { p: RelayPublication }) {
  const { t } = useTranslation("settings");
  return <>{publicationLabel(p, t)}</>;
}

/** A word for the upstream's state — never its id, which is a slug and
 *  sits behind the DevBadge beside the name. */
function PublicationState({ status, live }: { status: RelayStatus; live: RelayUpstreamLive | undefined }) {
  const { t } = useTranslation("settings");
  if (live?.error) return <>{live.error}</>;
  if (live?.ready) return <>{t("relay.upstreamReady")}</>;
  // Live state exists only for a relay this process serves. A terminal
  // relay's upstreams are unknown here — it may be refusing this one —
  // so the row says that, not that it is served.
  if (status.runningHere) return <>{t("relay.upstreamNotReady")}</>;
  if (status.serving) return <>{t("relay.upstreamElsewhere")}</>;
  return <>{t("relay.upstreamStopped")}</>;
}

/** An account and whether it may be published. An ineligible account
 *  stays in the list with its reason, rather than vanishing. */
function AccountRow({
  a,
  published,
  disabled,
  publishing,
  onPublish,
}: {
  a: RelayAccount;
  published: boolean;
  disabled: boolean;
  /** This row's Publish is the change in flight. */
  publishing: boolean;
  onPublish: () => void;
}) {
  const { t } = useTranslation("settings");
  return (
    <li>
      <span style={ROW_NAME}>{a.email}</span>
      {published ? (
        <span className="mono-cap" style={ROW_META}>
          {t("relay.isPublished")}
        </span>
      ) : a.ineligible ? (
        <span style={{ fontSize: "var(--fs-xs)", color: "var(--fg-faint)" }}>
          {t(INELIGIBLE_LABEL[a.ineligible])}
        </span>
      ) : (
        <Button variant="ghost" disabled={disabled} onClick={onPublish}>
          {publishing ? t("relay.publishing") : t("relay.publish")}
        </Button>
      )}
    </li>
  );
}
