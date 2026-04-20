import { type ReactNode, useEffect, useId, useState } from "react";
import { api } from "../../api";
import { Avatar, avatarColorFor } from "../../components/primitives/Avatar";
import { Button } from "../../components/primitives/Button";
import { Glyph } from "../../components/primitives/Glyph";
import {
  Modal,
  ModalBody,
  ModalFooter,
  ModalHeader,
} from "../../components/primitives/Modal";
import { Tag } from "../../components/primitives/Tag";
import { useFocusTrap } from "../../hooks/useFocusTrap";
import { NF } from "../../icons";
import type { AccountSummary } from "../../types";
import { ActionCard } from "./ActionCard";

interface AddAccountModalProps {
  open: boolean;
  onClose: () => void;
  onAdded: (email: string) => void;
  onError: (message: string) => void;
  /** Registered accounts — used to flag the "known" state. */
  accounts: AccountSummary[];
}

/**
 * Three-state add-account flow, driven by `currentCcIdentity()`:
 *
 *   • `new`     — CC is signed in as a brand-new email. Import is enabled.
 *   • `known`   — CC is signed in as an email we already manage.
 *   • `empty`   — CC has no blob at all.
 *   • `error`   — /profile failed; surface the error but keep both actions
 *                 dimmed (we can't trust the state).
 *
 * Action 1 maps to `api.accountAddFromCurrent()`. Action 2 ("Log in with
 * a new account") remains disabled until the backend exposes a
 * `register_from_browser` command — the existing `accountLogin(uuid)`
 * path is for RE-logging an existing account, not creating a fresh
 * slot.
 */
type Preflight =
  | { kind: "checking" }
  | { kind: "new"; email: string; orgName: string | null; subscription: string | null }
  | { kind: "known"; email: string; knownAccount: AccountSummary }
  | { kind: "empty" }
  | { kind: "error"; message: string };

export function AddAccountModal({
  open,
  onClose,
  onAdded,
  onError,
  accounts,
}: AddAccountModalProps) {
  const [importing, setImporting] = useState(false);
  const [browserLoggingIn, setBrowserLoggingIn] = useState(false);
  const [preflight, setPreflight] = useState<Preflight>({ kind: "checking" });
  const trapRef = useFocusTrap<HTMLDivElement>();
  const titleId = useId();

  useEffect(() => {
    if (!open) return;
    setImporting(false);
    setPreflight({ kind: "checking" });
    let cancelled = false;

    (async () => {
      try {
        const identity = await api.currentCcIdentity();
        if (cancelled) return;
        if (identity.error) {
          setPreflight({ kind: "error", message: identity.error });
          return;
        }
        if (!identity.email) {
          setPreflight({ kind: "empty" });
          return;
        }
        const known = accounts.find(
          (a) => a.email.toLowerCase() === identity.email!.toLowerCase(),
        );
        if (known) {
          setPreflight({
            kind: "known",
            email: identity.email,
            knownAccount: known,
          });
        } else {
          setPreflight({
            kind: "new",
            email: identity.email,
            orgName: null,
            subscription: null,
          });
        }
      } catch (e) {
        if (cancelled) return;
        setPreflight({
          kind: "error",
          message: e instanceof Error ? e.message : String(e),
        });
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [open, accounts]);

  const handleImport = async () => {
    setImporting(true);
    try {
      const outcome = await api.accountAddFromCurrent();
      onAdded(outcome.email);
      onClose();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setImporting(false);
    }
  };

  const handleBrowserLogin = async () => {
    setBrowserLoggingIn(true);
    try {
      const outcome = await api.accountRegisterFromBrowser();
      onAdded(outcome.email);
      onClose();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setBrowserLoggingIn(false);
    }
  };

  const summary = summaryFor(preflight);

  return (
    <Modal
      open={open}
      onClose={onClose}
      width="lg"
      aria-labelledby={titleId}
    >
      <div ref={trapRef} style={{ display: "contents" }}>
        <ModalHeader
          glyph={NF.plus}
          title="Add account"
          onClose={onClose}
          id={titleId}
        />

        <ModalBody>
          {/* preflight summary strip */}
          <div
            style={{
              display: "flex",
              gap: "var(--sp-10)",
              alignItems: "center",
              fontSize: "var(--fs-sm)",
              color: "var(--fg-muted)",
              padding: "var(--sp-10) var(--sp-12)",
              borderRadius: "var(--r-2)",
              background: "var(--bg-sunken)",
              border: "var(--bw-hair) solid var(--line)",
              marginBottom: "var(--sp-16)",
            }}
            aria-live="polite"
          >
            <Glyph
              g={summary.glyph}
              style={{ color: summary.tone, flexShrink: 0 }}
            />
            <div
              style={{ flex: 1, lineHeight: "var(--lh-body)" }}
            >
              {summary.text}
            </div>
          </div>

          {/* Action 1 — Import from Claude Code */}
          <ActionCard
            glyph={NF.download}
            title="Import from Claude Code"
            subtitle={importSubtitle(preflight)}
            command="account_add_from_current"
            accent
            disabled={preflight.kind !== "new" || importing}
            onClick={handleImport}
            cta={importing ? "Importing…" : "Import"}
            ctaGlyph={importing ? NF.clock : NF.download}
          >
            {preflight.kind === "new" && (
              <IdentityPreview
                email={preflight.email}
                subscription={preflight.subscription}
                orgName={preflight.orgName}
              />
            )}
            {preflight.kind === "known" && (
              <IdentityPreview
                email={preflight.email}
                subscription={preflight.knownAccount.subscription_type}
                orgName={preflight.knownAccount.org_name}
                dimmed
                badge={<Tag tone="neutral">already managed</Tag>}
              />
            )}
          </ActionCard>

          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--sp-10)",
              margin: "var(--sp-14) 0",
              color: "var(--fg-ghost)",
              fontSize: "var(--fs-xs)",
            }}
            aria-hidden
          >
            <div
              style={{
                flex: 1,
                height: "var(--bw-hair)",
                background: "var(--line)",
              }}
            />
            <span className="mono-cap">or add a different account</span>
            <div
              style={{
                flex: 1,
                height: "var(--bw-hair)",
                background: "var(--line)",
              }}
            />
          </div>

          {/* Action 2 — Browser login. Wired through
              account_register_from_browser; the core service owns the
              subprocess so the refresh token never enters JS. */}
          <ActionCard
            glyph={NF.user}
            title="Log in with a new account…"
            subtitle={
              browserLoggingIn
                ? "Waiting for you to finish in the browser…"
                : "Open a browser, complete OAuth, then register the result as a fresh account."
            }
            command="account_register_from_browser"
            disabled={browserLoggingIn || importing}
            onClick={handleBrowserLogin}
            cta={browserLoggingIn ? "Waiting…" : "Log in"}
            ctaGlyph={browserLoggingIn ? NF.clock : NF.arrowUpR}
          />
        </ModalBody>

        <ModalFooter>
          <Button variant="ghost" onClick={onClose}>
            Close
          </Button>
        </ModalFooter>
      </div>
    </Modal>
  );
}

function summaryFor(p: Preflight): {
  glyph: string;
  tone: string;
  text: ReactNode;
} {
  switch (p.kind) {
    case "checking":
      return {
        glyph: NF.clock,
        tone: "var(--fg-faint)",
        text: "Checking Claude Code's current session…",
      };
    case "new":
      return {
        glyph: NF.check,
        tone: "var(--ok)",
        text: (
          <>
            Claude Code is signed in as{" "}
            <span style={{ color: "var(--fg)", fontWeight: 600 }}>
              {p.email}
            </span>
            .
          </>
        ),
      };
    case "known":
      return {
        glyph: NF.info,
        tone: "var(--fg-muted)",
        text: (
          <>
            Claude Code is signed in as{" "}
            <span style={{ color: "var(--fg)", fontWeight: 600 }}>
              {p.email}
            </span>{" "}
            — already managed.
          </>
        ),
      };
    case "empty":
      return {
        glyph: NF.warn,
        tone: "var(--warn)",
        text: "Claude Code has no saved credentials.",
      };
    case "error":
      return {
        glyph: NF.warn,
        tone: "var(--warn)",
        text: <>Couldn't read Claude Code session: {p.message}</>,
      };
  }
}

function importSubtitle(p: Preflight): string {
  switch (p.kind) {
    case "new":
      return "Copy the current credential blob into a new managed slot.";
    case "known":
      return "Nothing new to import — that account is already managed.";
    case "empty":
      return "No credentials to import. Sign in with Claude Code first.";
    case "error":
      return "Preflight failed. Resolve the error above and retry.";
    case "checking":
      return "Waiting on Claude Code session check…";
  }
}

function IdentityPreview({
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
