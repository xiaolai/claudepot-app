import { type ReactNode, useEffect, useId, useState } from "react";
import type { NfIcon } from "../../icons";
import { api } from "../../api";
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
import { useOperations } from "../../hooks/useOperations";
import { redactSecrets } from "../../lib/redactSecrets";
import { NF } from "../../icons";
import type { AccountSummary } from "../../types";
import { ActionCard } from "./ActionCard";
import { DesktopImportCard } from "./DesktopImportCard";
import { IdentityPreview } from "./IdentityPreview";
import { LOGIN_PHASES, renderLoginResult } from "./loginProgress";

interface AddAccountModalProps {
  open: boolean;
  onClose: () => void;
  onAdded: (email: string) => void;
  onError: (message: string) => void;
  /** Registered accounts — used to flag the "known" state. */
  accounts: AccountSummary[];
  /** Shared Desktop adopt action — the shell owns toasts/refresh/tray.
   *  Resolves to `true` iff the bind committed. */
  onAdoptDesktop: (account: AccountSummary) => Promise<boolean>;
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
  onAdoptDesktop,
}: AddAccountModalProps) {
  const [importing, setImporting] = useState(false);
  const [browserLoggingIn, setBrowserLoggingIn] = useState(false);
  const [preflight, setPreflight] = useState<Preflight>({ kind: "checking" });
  const trapRef = useFocusTrap<HTMLDivElement>();
  const titleId = useId();
  const { open: openOpModal } = useOperations();

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
          // Backend may interpolate user input or token bodies into
          // the error string before returning it as a structured
          // field — same reason the catch arm runs through redactSecrets.
          setPreflight({
            kind: "error",
            message: redactSecrets(identity.error),
          });
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
        // Backend errors can interpolate user input or token bodies;
        // run them through the UI redactor before they reach the DOM
        // or the toast pipeline.
        const raw = e instanceof Error ? e.message : String(e);
        setPreflight({ kind: "error", message: redactSecrets(raw) });
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
      const raw = e instanceof Error ? e.message : String(e);
      onError(redactSecrets(raw));
    } finally {
      setImporting(false);
    }
  };

  const handleBrowserLogin = async () => {
    setBrowserLoggingIn(true);
    try {
      // Async start: returns op_id immediately; phase events flow on
      // `op-progress::<op_id>`. The shell `OperationProgressModal`
      // takes over the user-visible surface.
      const opId = await api.accountRegisterFromBrowserStart();
      // Hand off the AddAccountModal — the OperationProgressModal owns
      // the in-flight surface from here. The parent shell will pick up
      // the new account on the next refresh once `onComplete` fires.
      openOpModal({
        opId,
        title: "Add account: browser login",
        phases: LOGIN_PHASES,
        fetchStatus: api.accountLoginStatus,
        renderResult: renderLoginResult,
        onComplete: () => {
          // We don't know the new email yet (the synchronous return
          // value is gone); the parent's refresh will pick it up. Pass
          // an empty string so the parent's contract holds.
          onAdded("");
        },
        onError: (detail) => {
          const msg = detail ?? "";
          if (!/cancel/i.test(msg)) {
            onError(msg ? redactSecrets(msg) : "register failed");
          }
        },
      });
      onClose();
    } catch (e) {
      // Cancelled flows produce `register failed: claude auth login was
      // cancelled by the user` from core. Suppress the toast in that
      // case — the user just clicked Cancel, they don't need a warning.
      const msg = e instanceof Error ? e.message : String(e);
      if (!/cancel/i.test(msg)) {
        onError(redactSecrets(msg));
      }
    } finally {
      setBrowserLoggingIn(false);
    }
  };

  const handleCancelBrowserLogin = async () => {
    // Fire-and-forget: the command never errors, and the awaited
    // `handleBrowserLogin` promise is what actually surfaces the
    // cancellation result back to state.
    try {
      await api.accountLoginCancel();
    } catch {
      // The backend treats "nothing running" as a no-op. Anything
      // else is the lock-poisoning corner case — not worth a toast
      // when the user is trying to back out of the flow.
    }
  };

  /**
   * Wrap every dismiss route (Esc / scrim / header-X / footer Close)
   * so we always cancel the backend before the modal disappears.
   * Without this, a close while `browserLoggingIn` leaves the
   * `claude auth login` subprocess running invisibly in the
   * background, which is exactly the original bug this feature was
   * meant to fix. Cancel is fire-and-forget so dismissal is still
   * instant from the user's perspective.
   */
  const handleRequestClose = () => {
    if (browserLoggingIn) {
      void handleCancelBrowserLogin();
    }
    onClose();
  };

  const summary = summaryFor(preflight);

  return (
    <Modal
      open={open}
      onClose={handleRequestClose}
      width="lg"
      aria-labelledby={titleId}
    >
      <div ref={trapRef} style={{ display: "contents" }}>
        <ModalHeader
          glyph={NF.plus}
          title="Add account"
          onClose={handleRequestClose}
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
              subprocess so the refresh token never enters JS. While the
              flow is waiting for the browser, the card disables itself
              and a prominent Cancel appears in the footer (plus a
              secondary Cancel inline for discoverability). */}
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
          >
            {browserLoggingIn && (
              <div
                style={{
                  marginTop: "var(--sp-10)",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "space-between",
                  gap: "var(--sp-10)",
                  padding: "var(--sp-8) var(--sp-10)",
                  borderRadius: "var(--r-2)",
                  background: "var(--bg-sunken)",
                  border: "var(--bw-hair) solid var(--line)",
                  fontSize: "var(--fs-xs)",
                  color: "var(--fg-muted)",
                }}
                aria-live="polite"
              >
                <span>
                  Finish sign-in in your browser, or stop waiting if you
                  already cancelled there.
                </span>
                <Button
                  variant="ghost"
                  glyph={NF.x}
                  onClick={handleCancelBrowserLogin}
                  aria-label="Cancel browser login"
                >
                  Cancel login
                </Button>
              </div>
            )}
          </ActionCard>

          {/* Tier 3-A — Desktop session import. Decrypts live
              oauth:tokenCache via the authoritative /profile path
              (strict probe) and offers a one-click Bind when it
              matches a registered account. Hidden behind the
              divider so the default focus stays on the CC flow. */}
          <DesktopImportCard
            accounts={accounts}
            externallyDisabled={importing || browserLoggingIn}
            onAdoptDesktop={onAdoptDesktop}
            onAdopted={(email) => {
              onAdded(email);
              onClose();
            }}
          />
        </ModalBody>

        <ModalFooter>
          {browserLoggingIn ? (
            <Button
              variant="solid"
              glyph={NF.x}
              onClick={handleCancelBrowserLogin}
            >
              Cancel login
            </Button>
          ) : (
            <Button variant="ghost" onClick={handleRequestClose}>
              Close
            </Button>
          )}
        </ModalFooter>
      </div>
    </Modal>
  );
}

function summaryFor(p: Preflight): {
  glyph: NfIcon;
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

