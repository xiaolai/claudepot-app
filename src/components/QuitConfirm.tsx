import { useEffect, useId, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Button } from "./primitives/Button";
import { Glyph } from "./primitives/Glyph";
import {
  Modal,
  ModalHeader,
  ModalBody,
  ModalFooter,
} from "./primitives/Modal";
import { NF } from "../icons";
import { api } from "../api";

/**
 * Snapshot of one in-flight op, surfaced when the Rust quit gate
 * (`attempt_quit` in `src-tauri/src/app_menu.rs`) refuses an
 * immediate exit. Mirrors the `QuitGateOp` serde struct.
 */
type QuitGateOp = {
  op_id: string;
  kind: string;
  label: string;
};

/**
 * Quit-confirm modal. Listens on `cp-quit-requested` (emitted only when
 * `RunningOps` has live entries) and asks the user whether to abandon
 * those ops. "Quit anyway" calls `quit_now`, which exits the process;
 * "Stay" dismisses without further IPC.
 *
 * Mounted at the App level — `cp-quit-requested` is a global gesture
 * (⌘Q from anywhere, tray "Quit Claudepot" click) and shouldn't depend
 * on which section is mounted.
 */
export function QuitConfirm() {
  const [ops, setOps] = useState<QuitGateOp[] | null>(null);
  const titleId = useId();

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | null = null;
    listen<QuitGateOp[]>("cp-quit-requested", (event) => {
      // Empty payload would mean the backend's filter logic drifted —
      // fall back to closing rather than showing an empty modal.
      if (!event.payload || event.payload.length === 0) {
        setOps(null);
        return;
      }
      setOps(event.payload);
    })
      .then((fn) => {
        if (!active) fn();
        else unlisten = fn;
      })
      .catch((err: unknown) => {
        // Subscription failure makes the quit gate undetectable from
        // the renderer — log so the cause is diagnosable. Without
        // this, ⌘Q would silently no-op while the backend keeps
        // emitting `cp-quit-requested`.
        console.error("QuitConfirm: listen(cp-quit-requested) failed:", err);
      });
    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  if (!ops) return null;

  const onCancel = () => setOps(null);
  const onConfirm = () => {
    // `quit_now` exits the process and never resolves on the success
    // path; only an IPC-layer failure would surface here. Log and
    // leave the modal up so a retry is possible.
    api.quitNow().catch((err: unknown) => {
      console.error("quit_now failed:", err);
    });
  };

  const count = ops.length;
  const heading =
    count === 1 ? "1 operation in progress" : `${count} operations in progress`;

  return (
    <Modal
      open
      onClose={onCancel}
      aria-labelledby={titleId}
      closeOnBackdrop={false}
    >
      <ModalHeader
        glyph={NF.warn}
        title={heading}
        id={titleId}
        onClose={onCancel}
      />
      <ModalBody>
        <p style={{ marginTop: 0 }}>
          Quitting now will abandon the work below. Repairable operations
          (project rename, repair) leave a journal entry you can resume
          later from Projects → Repair; one-shot operations (verify,
          login, share) will need to be restarted.
        </p>
        <ul
          style={{
            listStyle: "none",
            padding: 0,
            margin: 0,
            display: "flex",
            flexDirection: "column",
            gap: "var(--sp-4)",
          }}
        >
          {ops.map((op) => (
            <li
              key={op.op_id}
              style={{
                display: "flex",
                alignItems: "center",
                gap: "var(--sp-8)",
                padding: "var(--sp-6) var(--sp-8)",
                border: "var(--bw-hair) solid var(--line)",
                borderRadius: "var(--r-1)",
                fontSize: "var(--fs-sm)",
              }}
            >
              <Glyph g={NF.refresh} />
              <span>{op.label}</span>
            </li>
          ))}
        </ul>
      </ModalBody>
      <ModalFooter>
        <Button variant="ghost" onClick={onCancel} autoFocus>
          Stay
        </Button>
        <Button variant="solid" danger onClick={onConfirm}>
          Quit anyway
        </Button>
      </ModalFooter>
    </Modal>
  );
}
