import { useEffect, useRef, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { Modal } from "../../components/primitives/Modal";
import { api } from "../../api";
import { renderError } from "../../lib/i18n-error";
import { i18n } from "../../lib/i18n";
import { useAppState } from "../../providers/AppStateProvider";
import type {
  RouteCreateDto,
  RouteDetailsDto,
  RouteSummaryDto,
  RouteUpdateDto,
} from "../../types";
import { RouteForm } from "./RouteForm";

interface AddRouteModalProps {
  open: boolean;
  onClose: () => void;
  onCreated: (route: RouteSummaryDto) => void;
}

export function AddRouteModal({ open, onClose, onCreated }: AddRouteModalProps) {
  const { t } = useTranslation("providers");
  const { pushToast } = useAppState();
  return (
    <Modal open={open} onClose={onClose} width="lg" aria-labelledby="add-route-title">
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "var(--sp-12)",
          padding: "var(--sp-20) var(--sp-24)",
          maxHeight: "var(--modal-body-cap-lg)",
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
              color: "var(--fg)",
            }}
          >
            {t("addModal.title")}
          </h2>
          <p
            style={{
              margin: "var(--sp-4) 0 0",
              fontSize: "var(--fs-sm)",
              color: "var(--fg-faint)",
            }}
          >
            {t("addModal.subtitle")}
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
              pushToast("error", renderError(e, t("addModal.addFailed")));
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
  /**
   * The summary the user clicked Edit on. The modal fetches the
   * full `RouteDetailsDto` via `routes_get` on open so the form
   * can hydrate every provider-specific field (the summary alone
   * is too thin for non-gateway providers).
   */
  initialSummary: RouteSummaryDto | null;
  onClose: () => void;
  onSaved: (route: RouteSummaryDto) => void;
}

export function EditRouteModal({
  open,
  initialSummary,
  onClose,
  onSaved,
}: EditRouteModalProps) {
  const { t } = useTranslation("providers");
  const { pushToast } = useAppState();
  const [details, setDetails] = useState<RouteDetailsDto | null>(null);
  const [loading, setLoading] = useState(false);

  // `onClose` is recreated on every parent render (ThirdPartySection
  // re-renders on a timer via the live Activity strip). It must NOT
  // be a fetch-effect dep: re-running the effect flips `loading`
  // true, which unmounts RouteForm and remounts it on refetch —
  // discarding the user's edits and scroll position. `onClose` is a
  // genuine parent-owned action the effect must still be able to
  // invoke (on load failure), so hold it in a ref — the correct
  // primitive for a non-reactive callback used inside an effect.
  const onCloseRef = useRef(onClose);
  useEffect(() => {
    onCloseRef.current = onClose;
  });

  const routeId = initialSummary?.id ?? null;
  useEffect(() => {
    if (!open || !routeId) {
      setDetails(null);
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    void api
      .routesGet(routeId)
      .then((d) => {
        if (!cancelled) {
          setDetails(d);
          setLoading(false);
        }
      })
      .catch((e) => {
        if (!cancelled) {
          setLoading(false);
          // `i18n.t`, not the hook's `t` — same reason `onClose`
          // sits in a ref above. `t`'s identity changes on a
          // language switch, so listing it as a dep would re-run
          // this fetch and discard the user's in-progress edits.
          // A toast resolves at fire time either way.
          pushToast(
            "error",
            renderError(e, i18n.t("editModal.loadFailed", { ns: "providers" })),
          );
          onCloseRef.current();
        }
      });
    return () => {
      cancelled = true;
    };
  }, [open, routeId, pushToast]);

  if (!initialSummary) return null;

  return (
    <Modal open={open} onClose={onClose} width="lg" aria-labelledby="edit-route-title">
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "var(--sp-12)",
          padding: "var(--sp-20) var(--sp-24)",
          maxHeight: "var(--modal-body-cap-lg)",
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
              color: "var(--fg)",
            }}
          >
            {t("editModal.title")}
          </h2>
          <p
            style={{
              margin: "var(--sp-4) 0 0",
              fontSize: "var(--fs-sm)",
              color: "var(--fg-faint)",
            }}
          >
            <Trans
              ns="providers"
              i18nKey="editModal.subtitle"
              values={{ name: initialSummary.name }}
              components={{ code: <code /> }}
            />
          </p>
        </header>
        {loading || !details ? (
          <p style={{ color: "var(--fg-faint)" }}>{t("editModal.loading")}</p>
        ) : (
          <RouteForm
            mode="edit"
            initial={details}
            onCancel={onClose}
            onSubmit={async (payload) => {
              try {
                const updated = await api.routesEdit(payload as RouteUpdateDto);
                onSaved(updated);
                onClose();
              } catch (e) {
                pushToast("error", renderError(e, t("editModal.editFailed")));
                throw e;
              }
            }}
          />
        )}
      </div>
    </Modal>
  );
}
