import { type MouseEvent } from "react";
import { Glyph } from "../../components/primitives/Glyph";
import { Input } from "../../components/primitives/Input";
import { NF } from "../../icons";
import type { AccountSummary, UsageMap } from "../../types";
import { AccountCard } from "./AccountCard";

interface Props {
  accounts: AccountSummary[];
  shown: AccountSummary[];
  usage: UsageMap;
  busyKeys: Set<string>;
  filter: string;
  onFilterChange: (value: string) => void;
  onRemove: (a: AccountSummary) => void;
  onLogin: (a: AccountSummary) => void;
  onContextMenu: (e: MouseEvent, a: AccountSummary) => void;
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
  busyKeys,
  filter,
  onFilterChange,
  onRemove,
  onLogin,
  onContextMenu,
}: Props) {
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
          gap: "var(--sp-16)",
          alignContent: "start",
        }}
      >
        {shown.map((a) => (
          <AccountCard
            key={a.uuid}
            account={a}
            usageEntry={usage[a.uuid] ?? null}
            loginBusy={busyKeys.has(`re-${a.uuid}`)}
            onRemove={onRemove}
            onLogin={onLogin}
            onContextMenu={onContextMenu}
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
              gap: "var(--sp-10)",
              alignItems: "center",
            }}
          >
            <Glyph g={NF.users} size="var(--sp-32)" color="var(--fg-ghost)" />
            <p style={{ margin: 0 }}>No accounts yet.</p>
            <p
              style={{
                margin: 0,
                fontSize: "var(--fs-xs)",
                color: "var(--fg-faint)",
              }}
            >
              {"Click "}
              <b>Add account</b>
              {" to import Claude Code's current session."}
            </p>
          </div>
        )}
      </div>
    </>
  );
}
