import { Modal } from "../../components/primitives/Modal";
import { api } from "../../api";
import type { RouteCreateDto, RouteSummaryDto, RouteUpdateDto } from "../../types";
import { RouteForm } from "./RouteForm";

interface AddRouteModalProps {
  open: boolean;
  onClose: () => void;
  onCreated: (route: RouteSummaryDto) => void;
  onError: (msg: string) => void;
}

export function AddRouteModal({
  open,
  onClose,
  onCreated,
  onError,
}: AddRouteModalProps) {
  return (
    <Modal open={open} onClose={onClose} width="lg" aria-labelledby="add-route-title">
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "var(--sp-12)",
          padding: "var(--sp-20) var(--sp-24)",
          maxHeight: "min(85vh, 720px)",
          overflowY: "auto",
        }}
      >
        <header>
          <h2
            id="add-route-title"
            style={{
              margin: 0,
              fontSize: "var(--fs-lg)",
              fontWeight: 600,
              color: "var(--fg-strong)",
            }}
          >
            Add a third-party route
          </h2>
          <p
            style={{
              margin: "var(--sp-4) 0 0",
              fontSize: "var(--fs-sm)",
              color: "var(--fg-faint)",
            }}
          >
            Configure a non-Anthropic LLM backend. Picks the provider
            type below; per-provider fields appear inline.
          </p>
        </header>
        <RouteForm
          mode="add"
          onCancel={onClose}
          onSubmit={async (payload) => {
            try {
              const created = await api.routesAdd(payload as RouteCreateDto);
              onCreated(created);
              onClose();
            } catch (e) {
              onError(`Add route failed: ${e instanceof Error ? e.message : e}`);
              throw e;
            }
          }}
        />
      </div>
    </Modal>
  );
}

export interface EditRouteModalProps {
  open: boolean;
  initial: RouteSummaryDto | null;
  onClose: () => void;
  onSaved: (route: RouteSummaryDto) => void;
  onError: (msg: string) => void;
}

export function EditRouteModal({
  open,
  initial,
  onClose,
  onSaved,
  onError,
}: EditRouteModalProps) {
  if (!initial) return null;
  return (
    <Modal open={open} onClose={onClose} width="lg" aria-labelledby="edit-route-title">
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "var(--sp-12)",
          padding: "var(--sp-20) var(--sp-24)",
          maxHeight: "min(85vh, 720px)",
          overflowY: "auto",
        }}
      >
        <header>
          <h2
            id="edit-route-title"
            style={{
              margin: 0,
              fontSize: "var(--fs-lg)",
              fontWeight: 600,
              color: "var(--fg-strong)",
            }}
          >
            Edit route
          </h2>
          <p
            style={{
              margin: "var(--sp-4) 0 0",
              fontSize: "var(--fs-sm)",
              color: "var(--fg-faint)",
            }}
          >
            Editing <code>{initial.name}</code>. Secret fields are blank
            for safety — leave blank to keep the existing values, or
            type to replace.
          </p>
        </header>
        <RouteForm
          mode="edit"
          initial={initial}
          onCancel={onClose}
          onSubmit={async (payload) => {
            try {
              const updated = await api.routesEdit(payload as RouteUpdateDto);
              onSaved(updated);
              onClose();
            } catch (e) {
              onError(`Edit failed: ${e instanceof Error ? e.message : e}`);
              throw e;
            }
          }}
        />
      </div>
    </Modal>
  );
}
