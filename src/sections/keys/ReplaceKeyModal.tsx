import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import {
  Modal,
  ModalBody,
  ModalFooter,
  ModalHeader,
} from "../../components/primitives/Modal";
import { renderError } from "../../lib/i18n-error";

/**
 * Rotate a stored key's secret, keeping the row.
 *
 * Deliberately not "remove then add": that mints a new uuid and resets
 * `created_at`, destroying the answer to "how long has this credential
 * been in service" at exactly the moment the user is asking it. Here
 * the label, account binding and age all survive; only the secret and
 * its preview change.
 *
 * The token state is cleared in a `finally` on submit so the secret
 * does not outlive the single bridge call in React state — the same
 * contract `AddKeyModal` follows (see `rules/architecture.md`, "IPC
 * trust + secret direction").
 */
export function ReplaceKeyModal({
  kind,
  uuid,
  label,
  onClose,
  onReplaced,
}: {
  kind: "api" | "oauth";
  uuid: string;
  label: string;
  onClose: () => void;
  onReplaced: () => void;
}) {
  const { t } = useTranslation("keys");
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Belt and braces: if the modal unmounts by any path other than
  // submit (Esc, backdrop, parent re-render), drop the secret too.
  useEffect(() => () => setToken(""), []);

  const onSubmit = useCallback(async () => {
    if (!token.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      if (kind === "api") await api.keyApiUpdateSecret(uuid, token);
      else await api.keyOauthUpdateSecret(uuid, token);
      onReplaced();
      onClose();
    } catch (e) {
      setError(renderError(e, t("replace.failed")));
    } finally {
      setToken("");
      setBusy(false);
    }
  }, [busy, kind, onClose, onReplaced, t, token, uuid]);

  return (
    <Modal open onClose={onClose} width="md">
      <ModalHeader title={t("replace.title", { label })} onClose={onClose} />
      <ModalBody>
        <p style={{ marginTop: 0, color: "var(--fg-muted)" }}>
          {t("replace.description")}
        </p>
        <input
          type="password"
          autoFocus
          value={token}
          onChange={(e) => setToken(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void onSubmit();
          }}
          placeholder={kind === "api" ? "sk-ant-api03-…" : "sk-ant-oat01-…"}
          aria-label={t("replace.inputAria", { label })}
          style={{ width: "100%", fontFamily: "var(--font-mono)" }}
        />
        {/* Disabled buttons state their reason inline, per
            rules/design.md — never only in a tooltip. */}
        {!token.trim() && !error ? (
          <p style={{ color: "var(--fg-faint)", fontSize: "var(--fs-xs)" }}>
            {t("replace.needToken")}
          </p>
        ) : null}
        {error ? (
          <p role="alert" style={{ color: "var(--danger)" }}>
            {error}
          </p>
        ) : null}
      </ModalBody>
      <ModalFooter>
        <Button onClick={onClose} disabled={busy}>
          {t("replace.cancel")}
        </Button>
        <Button
          variant="solid"
          onClick={onSubmit}
          disabled={busy || !token.trim()}
        >
          {busy ? t("replace.working") : t("replace.confirm")}
        </Button>
      </ModalFooter>
    </Modal>
  );
}
