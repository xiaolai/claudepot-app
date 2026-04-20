import { type ReactNode } from "react";
import { Avatar, avatarColorFor } from "../../components/primitives/Avatar";
import { Tag } from "../../components/primitives/Tag";
import { NF } from "../../icons";

/**
 * Compact identity card rendered inside AddAccountModal's ActionCard
 * bodies. Shows the avatar + email + plan badge + org name, optionally
 * dimmed for the "already managed" state.
 */
export function IdentityPreview({
  email,
  subscription,
  orgName,
  dimmed,
  badge,
}: {
  email: string;
  subscription: string | null;
  orgName: string | null;
  dimmed?: boolean;
  badge?: ReactNode;
}) {
  return (
    <div
      style={{
        marginTop: "var(--sp-10)",
        padding: "var(--sp-10) var(--sp-12)",
        background: "var(--bg)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-10)",
        opacity: dimmed ? "var(--opacity-quiet)" : 1,
      }}
    >
      <Avatar name={email} color={avatarColorFor(email)} size="lg" />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div
          style={{
            fontSize: "var(--fs-sm)",
            fontWeight: 600,
            display: "flex",
            gap: "var(--sp-8)",
            alignItems: "center",
            overflow: "hidden",
          }}
        >
          <span
            style={{
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {email}
          </span>
          {subscription && (
            <span
              className="mono-cap"
              style={{
                fontSize: "var(--fs-2xs)",
                color: "var(--fg-ghost)",
              }}
            >
              {subscription}
            </span>
          )}
        </div>
        {orgName && (
          <div
            style={{
              fontSize: "var(--fs-xs)",
              color: "var(--fg-faint)",
              marginTop: "var(--sp-px)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {orgName}
          </div>
        )}
      </div>
      {badge ?? (
        <Tag tone="ok" glyph={NF.check}>
          verified
        </Tag>
      )}
    </div>
  );
}
