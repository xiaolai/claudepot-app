import { useEffect, useRef, useState } from "react";
import type { NfIcon } from "../icons";
import { Avatar, avatarColorFor } from "../components/primitives/Avatar";
import { Glyph } from "../components/primitives/Glyph";
import { NF } from "../icons";
import type { AccountSummary } from "../types";

export type SwapTargetId = "cli" | "desktop";

interface SwapTarget {
  id: SwapTargetId;
  label: string;
  glyph: NfIcon;
}

interface SidebarTargetSwitcherProps {
  target: SwapTarget;
  accounts: AccountSummary[];
  /** UUID of the currently bound account, or null if unbound. */
  boundUuid: string | null;
  onBind: (target: SwapTargetId, uuid: string) => void;
  onManage: () => void;
}

/**
 * One of the two top "swap target" cards in the sidebar. Click to
 * open a dropdown listing every registered account; pick one to bind
 * that target. "Manage accounts…" jumps to the Accounts screen.
 *
 * The visible mini-row shows the currently bound account (avatar +
 * name). An empty binding renders a muted "Not bound" hint.
 */
export function SidebarTargetSwitcher({
  target,
  accounts,
  boundUuid,
  onBind,
  onManage,
}: SidebarTargetSwitcherProps) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const bound = boundUuid
    ? accounts.find((a) => a.uuid === boundUuid) ?? null
    : null;

  useEffect(() => {
    if (!open) return;
    const onDocClick = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    const t = window.setTimeout(() => {
      document.addEventListener("mousedown", onDocClick);
    }, 0);
    window.addEventListener("keydown", onKey);
    return () => {
      window.clearTimeout(t);
      document.removeEventListener("mousedown", onDocClick);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div ref={rootRef} style={{ position: "relative" }}>
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-haspopup="menu"
        aria-expanded={open}
        style={{
          width: "100%",
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-10)",
          padding: "var(--sp-11) var(--sp-10)",
          background: open ? "var(--bg-active)" : "var(--bg-raised)",
          border: `var(--bw-hair) solid ${open ? "var(--accent-border)" : "var(--line)"}`,
          borderRadius: "var(--r-2)",
          textAlign: "left",
          cursor: "pointer",
        }}
      >
        {/* Lead glyph was --fs-xl (22px) in the Nerd Font era; an
            NF glyph renders at ~65% of its font-size so that was
            visually ~14px. Lucide SVGs fill the full box, so the
            same token-value reads 50% larger. --fs-md matches the
            prior visual weight against the 10/11px label stack. */}
        <Glyph
          g={target.glyph}
          color="var(--fg-muted)"
          style={{ fontSize: "var(--fs-md)" }}
        />
        <div
          style={{
            flex: 1,
            minWidth: 0,
            overflow: "hidden",
          }}
        >
          <div
            style={{
              fontSize: "var(--fs-2xs)",
              fontWeight: 600,
              letterSpacing: "var(--ls-wide)",
              textTransform: "uppercase",
              color: "var(--fg-faint)",
            }}
          >
            {target.label}
          </div>
          <div
            style={{
              fontSize: "var(--fs-xs)",
              fontWeight: 600,
              color: bound ? "var(--fg)" : "var(--fg-faint)",
              whiteSpace: "nowrap",
              overflow: "hidden",
              textOverflow: "ellipsis",
              display: "flex",
              alignItems: "center",
              gap: "var(--sp-6)",
            }}
          >
            {bound ? (
              <>
                <Avatar
                  name={bound.email}
                  color={avatarColorFor(bound.email)}
                  size="2xs"
                />
                {bound.email}
              </>
            ) : (
              "Not bound"
            )}
          </div>
        </div>
        <Glyph
          g={open ? NF.chevronU : NF.chevronD}
          color="var(--fg-ghost)"
          style={{ fontSize: "var(--fs-2xs)" }}
        />
      </button>

      {open && (
        <div
          role="menu"
          style={{
            position: "absolute",
            top: `calc(100% + var(--sp-4))`,
            left: 0,
            right: 0,
            zIndex: "var(--z-popover)" as unknown as number,
            background: "var(--bg-raised)",
            border: "var(--bw-hair) solid var(--line-strong)",
            borderRadius: "var(--r-2)",
            boxShadow: "var(--shadow-popover)",
            overflow: "hidden",
          }}
        >
          <div
            style={{
              padding: "var(--sp-8) var(--sp-10) var(--sp-4)",
              fontSize: "var(--fs-2xs)",
              letterSpacing: "var(--ls-wide)",
              textTransform: "uppercase",
              color: "var(--fg-faint)",
              display: "flex",
              alignItems: "center",
              gap: "var(--sp-6)",
            }}
          >
            <Glyph g={target.glyph} style={{ fontSize: "var(--fs-2xs)" }} />
            Switch {target.label} to
          </div>
          <div style={{ padding: "var(--sp-4)" }}>
            {accounts.length === 0 ? (
              <div
                style={{
                  padding: "var(--sp-8) var(--sp-10)",
                  fontSize: "var(--fs-xs)",
                  color: "var(--fg-faint)",
                }}
              >
                No accounts registered yet.
              </div>
            ) : (
              accounts.map((a) => (
                <TargetSwitchOption
                  key={a.uuid}
                  account={a}
                  current={a.uuid === boundUuid}
                  onClick={() => {
                    onBind(target.id, a.uuid);
                    setOpen(false);
                  }}
                />
              ))
            )}
          </div>
          <div
            style={{
              borderTop: "var(--bw-hair) solid var(--line)",
              padding: "var(--sp-4)",
            }}
          >
            <button
              type="button"
              onClick={() => {
                onManage();
                setOpen(false);
              }}
              style={{
                width: "100%",
                display: "flex",
                alignItems: "center",
                gap: "var(--sp-8)",
                padding: "var(--sp-6) var(--sp-8)",
                fontSize: "var(--fs-xs)",
                color: "var(--fg-muted)",
                borderRadius: "var(--r-1)",
                background: "transparent",
                cursor: "pointer",
                textAlign: "left",
              }}
            >
              <Glyph g={NF.sliders} style={{ fontSize: "var(--fs-xs)" }} />
              <span>Manage accounts…</span>
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

interface TargetSwitchOptionProps {
  account: AccountSummary;
  current: boolean;
  onClick: () => void;
}

function TargetSwitchOption({
  account,
  current,
  onClick,
}: TargetSwitchOptionProps) {
  const [hover, setHover] = useState(false);
  return (
    <button
      type="button"
      onClick={current ? undefined : onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      aria-current={current || undefined}
      style={{
        width: "100%",
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-10)",
        padding: "var(--sp-7) var(--sp-8)",
        fontSize: "var(--fs-sm)",
        color: "var(--fg)",
        background: current
          ? "var(--accent-soft)"
          : hover
            ? "var(--bg-hover)"
            : "transparent",
        border: `var(--bw-hair) solid ${current ? "var(--accent-border)" : "transparent"}`,
        borderRadius: "var(--r-1)",
        textAlign: "left",
        cursor: current ? "default" : "pointer",
      }}
    >
      <Avatar
        name={account.email}
        color={avatarColorFor(account.email)}
        size="xs"
      />
      <div style={{ flex: 1, minWidth: 0, overflow: "hidden" }}>
        <div
          style={{
            fontSize: "var(--fs-sm)",
            fontWeight: 500,
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
          }}
        >
          {account.email}
        </div>
        {account.org_name && (
          <div
            style={{
              fontSize: "var(--fs-2xs)",
              color: "var(--fg-faint)",
              whiteSpace: "nowrap",
              overflow: "hidden",
              textOverflow: "ellipsis",
            }}
          >
            {account.org_name}
          </div>
        )}
      </div>
      {current ? (
        <Glyph
          g={NF.check}
          color="var(--accent)"
          style={{ fontSize: "var(--fs-xs)" }}
        />
      ) : hover ? (
        <Glyph
          g={NF.arrowR}
          color="var(--fg-faint)"
          style={{ fontSize: "var(--fs-xs)" }}
        />
      ) : null}
    </button>
  );
}
