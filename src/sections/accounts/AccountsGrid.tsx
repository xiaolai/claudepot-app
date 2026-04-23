import { type MouseEvent } from "react";
import { Button } from "../../components/primitives/Button";
import { Glyph } from "../../components/primitives/Glyph";
import { Input } from "../../components/primitives/Input";
import { NF } from "../../icons";
import type { AccountSummary, AppStatus, CcIdentity, UsageMap } from "../../types";
import { AccountCard } from "./AccountCard";
import type {
  CliTargetHandlers,
  DesktopTargetHandlers,
} from "./targetButtonStates";

interface Props {
  accounts: AccountSummary[];
  shown: AccountSummary[];
  usage: UsageMap;
  status: AppStatus;
  busyKeys: Set<string>;
  filter: string;
  onFilterChange: (value: string) => void;
  onLogin: (a: AccountSummary) => void;
  onContextMenu: (e: MouseEvent, a: AccountSummary) => void;
  cliHandlers: CliTargetHandlers;
  desktopHandlers: DesktopTargetHandlers;
  /** Claude Code's current signed-in identity, used to pre-fill an
   *  adopt CTA in the empty state. When present with an email and no
   *  error, the first-run prompt offers a single click to register
   *  that session as a Claudepot account. */
  ccIdentity?: CcIdentity | null;
  /** Fires the adopt flow — imports CC's current credentials into a
   *  new Claudepot account. Wired to `api.accountAddFromCurrent` by
   *  the section. */
  onAdoptCurrent?: () => void;
  /** Opens the AddAccountModal. Used by the empty state when no CC
   *  session is available to adopt. */
  onAdd: () => void;
}

/**
 * Filter bar + card grid + empty states for the Accounts section.
 * Extracted to keep the main section file under the loc-guardian
 * budget and to keep the empty-state copy next to the rendering code.
 */
export function AccountsGrid({
  accounts,
  shown,
  usage,
  status,
  busyKeys,
  filter,
  onFilterChange,
  onLogin,
  onContextMenu,
  cliHandlers,
  desktopHandlers,
  ccIdentity,
  onAdoptCurrent,
  onAdd,
}: Props) {
  // Pre-fill adoption when CC is already signed in. `error` null +
  // non-empty email covers the 0- or 1-account case where Claudepot
  // opens on a clean profile but the user's CLI is already authed.
  const ccSignedInEmail =
    ccIdentity && !ccIdentity.error && ccIdentity.email
      ? ccIdentity.email
      : null;
  return (
    <>
      {/* Filter input only earns its row when there are enough
          accounts to usefully narrow. With 1–3 accounts the input is
          pure chrome; once a 4th lands, the filter appears. */}
      {accounts.length > 3 && (
        <div
          style={{
            padding: "var(--sp-14) var(--sp-32)",
            borderBottom: "var(--bw-hair) solid var(--line)",
            display: "flex",
            gap: "var(--sp-12)",
            alignItems: "center",
            background: "var(--bg)",
          }}
        >
          <Input
            glyph={NF.search}
            placeholder="Filter accounts"
            value={filter}
            onChange={(e) => onFilterChange(e.target.value)}
            style={{ width: "var(--filter-input-width)" }}
            aria-label="Filter accounts"
          />
          {filter.trim() !== "" && (
            <span
              className="mono-cap"
              style={{
                color: "var(--fg-faint)",
                marginLeft: "var(--sp-4)",
              }}
            >
              {`${shown.length} / ${accounts.length}`}
            </span>
          )}
          <div style={{ flex: 1 }} />
        </div>
      )}

      <div
        style={{
          // Flex child of the shell's flex-column main. `flex: 1` +
          // `minHeight: 0` + `overflow: auto` keep scroll contained
          // here so ScreenHeader and the filter bar stay pinned
          // when the card list overflows.
          flex: 1,
          minHeight: 0,
          overflow: "auto",
          padding: "var(--sp-20) var(--sp-32) var(--sp-40)",
          display: "grid",
          gridTemplateColumns:
            "repeat(auto-fill, minmax(var(--content-cap-sm), 1fr))",
          // Items use `overflow: hidden` to clip border-radius, which
          // drops their min-content contribution to row sizing. Without
          // `max-content` the grid compresses auto rows to equal
          // shares of its height and clips card content. Explicit
          // `max-content` keeps rows at their intrinsic height so the
          // full UsageBlock + HealthFooter render; the grid's own
          // `overflow: auto` above handles pane-level scroll.
          gridAutoRows: "max-content",
          gap: "var(--sp-16)",
          alignContent: "start",
        }}
      >
        {shown.map((a) => (
          <AccountCard
            key={a.uuid}
            account={a}
            usageEntry={usage[a.uuid] ?? null}
            status={status}
            loginBusy={busyKeys.has(`re-${a.uuid}`)}
            onLogin={onLogin}
            onContextMenu={onContextMenu}
            cliHandlers={cliHandlers}
            desktopHandlers={desktopHandlers}
          />
        ))}
        {shown.length === 0 && accounts.length > 0 && (
          <div
            style={{
              gridColumn: "1 / -1",
              padding: "var(--sp-60)",
              textAlign: "center",
              color: "var(--fg-faint)",
              fontSize: "var(--fs-sm)",
            }}
          >
            No accounts match "{filter}".
          </div>
        )}
        {accounts.length === 0 && (
          <div
            style={{
              gridColumn: "1 / -1",
              padding: "var(--sp-60)",
              textAlign: "center",
              color: "var(--fg-faint)",
              fontSize: "var(--fs-sm)",
              display: "flex",
              flexDirection: "column",
              gap: "var(--sp-14)",
              alignItems: "center",
            }}
          >
            <Glyph g={NF.users} size="var(--sp-32)" color="var(--fg-ghost)" />
            <p
              style={{
                margin: 0,
                fontSize: "var(--fs-md)",
                color: "var(--fg)",
                fontWeight: 500,
              }}
            >
              No accounts yet.
            </p>
            {ccSignedInEmail ? (
              <>
                <p
                  style={{
                    margin: 0,
                    fontSize: "var(--fs-xs)",
                    color: "var(--fg-muted)",
                    maxWidth: "var(--content-cap-sm)",
                  }}
                >
                  Claude Code is already signed in as{" "}
                  <strong style={{ color: "var(--fg)" }}>
                    {ccSignedInEmail}
                  </strong>
                  . Adopt this session as your first Claudepot account?
                </p>
                <div
                  style={{
                    display: "flex",
                    gap: "var(--sp-8)",
                    alignItems: "center",
                  }}
                >
                  <Button
                    variant="solid"
                    glyph={NF.check}
                    onClick={onAdoptCurrent}
                  >
                    {`Adopt ${ccSignedInEmail}`}
                  </Button>
                  <Button variant="ghost" glyph={NF.plus} onClick={onAdd}>
                    Add a different account
                  </Button>
                </div>
              </>
            ) : (
              <>
                <p
                  style={{
                    margin: 0,
                    fontSize: "var(--fs-xs)",
                    color: "var(--fg-muted)",
                    maxWidth: "var(--content-cap-sm)",
                  }}
                >
                  Claudepot manages multiple Anthropic logins for Claude
                  Code and Claude Desktop. Sign in with a browser OAuth
                  flow to get started.
                </p>
                <Button variant="solid" glyph={NF.plus} onClick={onAdd}>
                  Add account
                </Button>
              </>
            )}
          </div>
        )}
      </div>
    </>
  );
}
