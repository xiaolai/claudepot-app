// Recovery modal for trash entries in MissingManifest /
// AbandonedStaging states. Asks the user to confirm the absolute
// target path + artifact kind before promoting a synthetic manifest.
//
// Replaces the previous `window.prompt` flow which violated
// `.claude/rules/design.md` ("Never use window.confirm/alert/prompt").

import { useId, useState } from "react";
import { Button } from "../../components/primitives/Button";
import { Input } from "../../components/primitives/Input";
import {
  Modal,
  ModalBody,
  ModalFooter,
  ModalHeader,
} from "../../components/primitives/Modal";
import type { LifecycleKind, TrashEntryDto } from "../../types";

const KINDS: LifecycleKind[] = ["skill", "agent", "command"];

export function RecoverDialog({
  entry,
  onCancel,
  onSubmit,
}: {
  entry: TrashEntryDto;
  onCancel: () => void;
  onSubmit: (target: string, kind: LifecycleKind) => void;
}) {
  const titleId = useId();
  const m = entry.manifest;
  const [target, setTarget] = useState<string>(m?.original_path ?? "");
  const [kind, setKind] = useState<LifecycleKind>(
    (m?.kind as LifecycleKind) ?? "agent",
  );
  // Recovery writes the synthesized manifest then asks the backend to
  // restore the payload at `target`. A relative path here is dangerous
  // — the backend would resolve it against its own cwd (the app
  // working directory), which is not where the user thinks they're
  // restoring. Require an absolute Unix or Windows-shaped path. The
  // backend revalidates this; the UI gate is the first line of
  // defense and the better UX (Recover stays disabled until valid).
  const trimmed = target.trim();
  const isAbsolute =
    trimmed.startsWith("/") ||
    trimmed.startsWith("\\\\") ||
    /^[A-Za-z]:[\\/]/.test(trimmed);
  const submittable = trimmed.length > 0 && isAbsolute;
  return (
    <Modal open onClose={onCancel} aria-labelledby={titleId}>
      <ModalHeader title="Recover trash entry" id={titleId} onClose={onCancel} />
      <ModalBody>
        <p
          style={{
            margin: "0 0 var(--sp-12)",
            fontSize: "var(--fs-sm)",
            color: "var(--fg-muted)",
          }}
        >
          This entry doesn't have a complete manifest. Confirm where it should
          be restored and what kind of artifact it is. The entry's payload
          contents are kept intact — only the destination is synthesized.
        </p>
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: "var(--sp-12)",
          }}
        >
          <label style={fieldLabelStyle()}>
            <span>Absolute target path</span>
            <Input
              value={target}
              onChange={(e) => setTarget(e.currentTarget.value)}
              placeholder="/Users/you/.claude/agents/foo.md"
              autoFocus
            />
            {trimmed.length > 0 && !isAbsolute && (
              <span
                style={{
                  fontSize: "var(--fs-2xs)",
                  color: "var(--danger)",
                  textTransform: "none",
                  letterSpacing: "normal",
                  marginTop: "var(--sp-2)",
                }}
              >
                Path must be absolute (starts with <code>/</code> or <code>C:\</code>).
              </span>
            )}
          </label>
          <label style={fieldLabelStyle()}>
            <span>Artifact kind</span>
            <select
              value={kind}
              onChange={(e) => setKind(e.currentTarget.value as LifecycleKind)}
              style={{
                padding: "var(--sp-6) var(--sp-8)",
                background: "var(--bg)",
                color: "var(--fg)",
                border: "var(--bw-hair) solid var(--line)",
                borderRadius: "var(--r-1)",
                fontSize: "var(--fs-sm)",
              }}
            >
              {KINDS.map((k) => (
                <option key={k} value={k}>
                  {k}
                </option>
              ))}
            </select>
          </label>
          {entry.state === "abandoned_staging" && (
            <p
              style={{
                margin: 0,
                fontSize: "var(--fs-2xs)",
                color: "var(--warn)",
                letterSpacing: "var(--ls-wide)",
                textTransform: "uppercase",
              }}
            >
              Abandoned staging — last interrupted op
            </p>
          )}
        </div>
      </ModalBody>
      <ModalFooter>
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button
          variant="solid"
          onClick={() => onSubmit(target.trim(), kind)}
          disabled={!submittable}
          autoFocus
        >
          Recover
        </Button>
      </ModalFooter>
    </Modal>
  );
}

function fieldLabelStyle(): React.CSSProperties {
  return {
    display: "flex",
    flexDirection: "column",
    gap: "var(--sp-4)",
    fontSize: "var(--fs-xs)",
    color: "var(--fg-muted)",
    letterSpacing: "var(--ls-wide)",
    textTransform: "uppercase",
  };
}
