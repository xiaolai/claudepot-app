import { useEffect, useId, useMemo, useState } from "react";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import { Input } from "../../components/primitives/Input";
import {
  Modal,
  ModalBody,
  ModalFooter,
  ModalHeader,
} from "../../components/primitives/Modal";
import { NF } from "../../icons";
import type { AccountSummaryBasic } from "../../types";

type DetectedKind = "api" | "oauth" | "invalid" | "empty";

function detect(token: string): DetectedKind {
  const t = token.trim();
  if (!t) return "empty";
  if (t.startsWith("sk-ant-api03-")) return "api";
  if (t.startsWith("sk-ant-oat01-")) return "oauth";
  return "invalid";
}

/**
 * Paste-first add form. Watches the token field, auto-detects API key
 * vs OAuth token by prefix, and shows the right helper copy. When the
 * user picks OAuth, an account dropdown appears — required per the
 * "tag at add-time" product decision.
 */
export function AddKeyModal({
  accounts,
  onClose,
  onAdded,
}: {
  accounts: AccountSummaryBasic[];
  onClose: () => void;
  onAdded: (kind: "api" | "oauth") => void;
}) {
  const [label, setLabel] = useState("");
  const [token, setToken] = useState("");
  const [accountUuid, setAccountUuid] = useState<string>("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const titleId = useId();

  const kind = detect(token);

  // Default the account pick to the active CLI account — every key is
  // created under some account; pre-filling the most likely one saves
  // a click in the common case.
  useEffect(() => {
    if (accountUuid) return;
    const active = accounts.find((a) => a.is_cli_active);
    if (active) setAccountUuid(active.uuid);
  }, [accountUuid, accounts]);

  const disableReason = useMemo(() => {
    if (!label.trim()) return "Label is required";
    if (kind === "empty") return "Paste a key or token";
    if (kind === "invalid")
      return "Must start with sk-ant-api03- or sk-ant-oat01-";
    if (!accountUuid) return "Pick the account this key was created under";
    return null;
  }, [label, kind, accountUuid]);

  const submit = async () => {
    if (disableReason) return;
    setBusy(true);
    setError(null);
    try {
      if (kind === "api") {
        await api.keyApiAdd(label.trim(), token.trim(), accountUuid);
        onAdded("api");
      } else if (kind === "oauth") {
        await api.keyOauthAdd(label.trim(), token.trim(), accountUuid);
        onAdded("oauth");
      }
    } catch (e) {
      setError(`${e}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      open
      onClose={busy ? undefined : onClose}
      width="md"
      aria-labelledby={titleId}
    >
      <ModalHeader
        glyph={NF.key}
        title="Add key"
        id={titleId}
        onClose={busy ? undefined : onClose}
      />
      <ModalBody>
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-14)" }}>
          <Field label="Label" hint="A short name you'll see in the list.">
            <Input
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              placeholder="e.g. Personal / CI / staging"
              autoFocus
              disabled={busy}
            />
          </Field>

          <Field
            label="Token"
            hint="Paste the full value. We detect the kind from its prefix."
          >
            <Input
              value={token}
              onChange={(e) => setToken(e.target.value)}
              placeholder="sk-ant-api03-… or sk-ant-oat01-…"
              disabled={busy}
              type="password"
            />
          </Field>

          {kind === "api" && (
            <Hint>
              <strong>API key</strong> detected.
            </Hint>
          )}
          {kind === "invalid" && (
            <Hint tone="danger">
              Unknown prefix. Expected <code>sk-ant-api03-</code> (API key) or{" "}
              <code>sk-ant-oat01-</code> (OAuth token).
            </Hint>
          )}

          <Field
            label="Created by account"
            hint={accountHint(kind, accounts.length)}
          >
            <AccountSelect
              accounts={accounts}
              value={accountUuid}
              onChange={setAccountUuid}
              disabled={busy}
            />
          </Field>

          {error && (
            <div
              role="alert"
              style={{
                padding: "var(--sp-8) var(--sp-10)",
                border: "var(--bw-hair) solid var(--danger)",
                borderRadius: "var(--r-2)",
                fontSize: "var(--fs-sm)",
                color: "var(--danger)",
              }}
            >
              {error}
            </div>
          )}
        </div>
      </ModalBody>
      <ModalFooter>
        <span
          style={{
            flex: 1,
            fontSize: "var(--fs-xs)",
            color: "var(--fg-faint)",
          }}
        >
          {disableReason ?? ""}
        </span>
        <Button variant="ghost" onClick={onClose} disabled={busy}>
          Cancel
        </Button>
        <Button
          variant="solid"
          onClick={() => void submit()}
          disabled={!!disableReason || busy}
        >
          {busy ? "Adding…" : "Add"}
        </Button>
      </ModalFooter>
    </Modal>
  );
}

function accountHint(kind: DetectedKind, total: number): React.ReactNode {
  if (total === 0) {
    return "No registered accounts yet — add one from the Accounts section first.";
  }
  if (kind === "oauth") {
    return (
      <>
        The account you were signed in as when you ran{" "}
        <code>claude setup-token</code>.
      </>
    );
  }
  if (kind === "api") {
    return "The Anthropic account that owns this API key in the console.";
  }
  return "Defaults to your active CLI account.";
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-4)" }}>
      <label
        style={{
          fontSize: "var(--fs-xs)",
          fontWeight: 500,
          color: "var(--fg-muted)",
        }}
      >
        {label}
      </label>
      {children}
      {hint && (
        <span
          style={{
            fontSize: "var(--fs-xs)",
            color: "var(--fg-faint)",
          }}
        >
          {hint}
        </span>
      )}
    </div>
  );
}

function Hint({
  tone,
  children,
}: {
  tone?: "accent" | "danger";
  children: React.ReactNode;
}) {
  const color =
    tone === "danger"
      ? "var(--danger)"
      : tone === "accent"
        ? "var(--accent-ink)"
        : "var(--fg-muted)";
  const bg =
    tone === "danger"
      ? "color-mix(in oklch, var(--danger) 8%, transparent)"
      : tone === "accent"
        ? "var(--accent-soft)"
        : "var(--bg-sunken)";
  return (
    <div
      style={{
        padding: "var(--sp-8) var(--sp-10)",
        background: bg,
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        fontSize: "var(--fs-sm)",
        color,
      }}
    >
      {children}
    </div>
  );
}

function AccountSelect({
  accounts,
  value,
  onChange,
  disabled,
}: {
  accounts: AccountSummaryBasic[];
  value: string;
  onChange: (v: string) => void;
  disabled?: boolean;
}) {
  return (
    <select
      value={value}
      onChange={(e) => onChange(e.target.value)}
      disabled={disabled}
      style={{
        height: "var(--input-height)",
        padding: "0 var(--sp-10)",
        background: "var(--bg-raised)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        fontFamily: "var(--font)",
        fontSize: "var(--fs-sm)",
        color: "var(--fg)",
      }}
    >
      {!value && <option value="">— select account —</option>}
      {accounts.map((a) => (
        <option key={a.uuid} value={a.uuid}>
          {a.email}
          {a.is_cli_active ? " · active CLI" : ""}
        </option>
      ))}
    </select>
  );
}
