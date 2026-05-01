import { useEffect, useState } from "react";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import { Modal } from "../../components/primitives/Modal";
import { FieldBlock } from "../../components/primitives/modalParts";
import type {
  ScheduleDto,
  ScheduleShapeName,
  TemplateDetailsDto,
  TemplateInstanceDto,
  TemplateRouteSummaryDto,
} from "../../types";
import { ScopeStatements } from "./ScopeStatements";
import { SchedulePicker } from "./SchedulePicker";
import { RoutePicker } from "./RoutePicker";
import { TemplateSampleReport } from "./TemplateSampleReport";

interface Props {
  open: boolean;
  templateId: string | null;
  onClose: () => void;
  onInstalled: () => void;
  onError: (msg: string) => void;
  onOpenThirdParties: () => void;
}

/**
 * Install flow for one template. Loads details + capable routes
 * on mount; presents scope statements, sample-report preview,
 * schedule picker, optional route picker, and an Install button.
 *
 * The dialog is `Modal` width="lg" — gives the sample report
 * room to breathe without forcing a separate full-screen page.
 */
export function TemplateInstallDialog({
  open,
  templateId,
  onClose,
  onInstalled,
  onError,
  onOpenThirdParties,
}: Props) {
  const [details, setDetails] = useState<TemplateDetailsDto | null>(null);
  const [routes, setRoutes] = useState<TemplateRouteSummaryDto[]>([]);
  const [routeId, setRouteId] = useState<string | null>(null);
  const [schedule, setSchedule] = useState<ScheduleDto | null>(null);
  const [sampleOpen, setSampleOpen] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open || !templateId) return;
    setDetails(null);
    setRoutes([]);
    setRouteId(null);
    setSchedule(null);
    setSampleOpen(false);
    setBusy(false);

    Promise.all([
      api.templatesGet(templateId),
      api.templatesCapableRoutes(templateId),
    ])
      .then(([d, rs]) => {
        setDetails(d);
        setRoutes(rs);
        setSchedule(initialSchedule(d.allowed_schedule_shapes, d));
      })
      .catch((e: unknown) => onError(String(e)));
  }, [open, templateId, onError]);

  if (!open || !templateId) return null;

  const ready = details !== null && schedule !== null;

  async function handleInstall() {
    if (!details || !schedule) return;
    setBusy(true);
    const instance: TemplateInstanceDto = {
      blueprint_id: details.summary.id,
      blueprint_schema_version: details.schema_version,
      placeholder_values: {}, // none of the v1 templates carry placeholders
      route_id: routeId ?? undefined,
      schedule,
    };
    try {
      await api.templatesInstall(instance);
      onInstalled();
      onClose();
    } catch (e: unknown) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      open={open}
      onClose={onClose}
      width="lg"
      aria-labelledby="template-install-title"
    >
      {!ready ? (
        <div
          style={{
            padding: "var(--sp-24)",
            color: "var(--fg-faint)",
            fontSize: "var(--fs-sm)",
          }}
        >
          Loading template…
        </div>
      ) : (
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: "var(--sp-16)",
            padding: "var(--sp-16) var(--sp-20)",
          }}
        >
          <div>
            <h2
              id="template-install-title"
              style={{
                margin: 0,
                fontSize: "var(--fs-lg)",
                color: "var(--fg)",
              }}
            >
              {details.summary.name}
            </h2>
            <p
              style={{
                margin: "var(--sp-4) 0 0",
                color: "var(--fg-2)",
                fontSize: "var(--fs-sm)",
              }}
            >
              {details.summary.tagline}
            </p>
          </div>

          <p
            style={{
              margin: 0,
              fontSize: "var(--fs-sm)",
              lineHeight: 1.6,
              color: "var(--fg)",
              whiteSpace: "pre-wrap",
            }}
          >
            {details.description}
          </p>

          <div>
            <button
              type="button"
              onClick={() => setSampleOpen((v) => !v)}
              style={{
                background: "none",
                border: "none",
                padding: 0,
                color: "var(--accent)",
                textDecoration: "underline",
                cursor: "pointer",
                fontSize: "var(--fs-sm)",
              }}
            >
              {sampleOpen ? "Hide" : "View"} sample report
            </button>
          </div>

          {sampleOpen && <TemplateSampleReport templateId={details.summary.id} />}

          <FieldBlock label="What this can do">
            <ScopeStatements scope={details.scope} />
          </FieldBlock>

          {schedule && (
            <FieldBlock label="When should it run?">
              <SchedulePicker
                allowedShapes={details.allowed_schedule_shapes}
                defaultShape={pickInitialShape(details.allowed_schedule_shapes)}
                defaultTime={extractDefaultTime(details)}
                defaultCron={details.default_schedule_cron}
                value={schedule}
                onChange={setSchedule}
              />
            </FieldBlock>
          )}

          <RoutePicker
            routes={routes}
            selectedRouteId={routeId}
            onChange={setRouteId}
            privacyClass={details.summary.privacy}
            onOpenThirdParties={onOpenThirdParties}
          />

          {details.summary.privacy === "any" && routeId !== null && (
            <p style={{ margin: 0, fontSize: "var(--fs-xs)", color: "var(--fg-faint)" }}>
              🌐 Sends data to your selected route.
            </p>
          )}

          <div
            style={{
              display: "flex",
              justifyContent: "flex-end",
              gap: "var(--sp-8)",
              paddingTop: "var(--sp-8)",
              borderTop: "var(--bw-hair) solid var(--line)",
            }}
          >
            <Button variant="ghost" onClick={onClose} disabled={busy}>
              Cancel
            </Button>
            <Button
              variant="solid"
              onClick={handleInstall}
              disabled={busy || installDisabled(details, routes)}
            >
              {busy ? "Installing…" : "Install"}
            </Button>
          </div>
        </div>
      )}
    </Modal>
  );
}

function installDisabled(
  details: TemplateDetailsDto,
  routes: TemplateRouteSummaryDto[],
): boolean {
  // Local-only template + no local route → can't install yet.
  if (
    details.summary.privacy === "local" &&
    !routes.some((r) => r.is_capable && r.is_local)
  ) {
    return true;
  }
  return false;
}

function pickInitialShape(shapes: ScheduleShapeName[]): ScheduleShapeName {
  // Prefer the most restrictive default the blueprint allows.
  for (const s of ["daily", "weekly", "weekdays", "hourly", "manual", "custom"] as const) {
    if (shapes.includes(s)) return s;
  }
  return "manual";
}

function initialSchedule(
  shapes: ScheduleShapeName[],
  details: TemplateDetailsDto,
): ScheduleDto {
  const time = extractDefaultTime(details);
  if (shapes.includes("daily")) return { kind: "daily", time };
  if (shapes.includes("weekly")) return { kind: "weekly", day: "mon", time };
  if (shapes.includes("weekdays")) return { kind: "weekdays", time };
  if (shapes.includes("hourly")) return { kind: "hourly", every_n_hours: 4 };
  if (shapes.includes("manual")) return { kind: "manual" };
  if (shapes.includes("custom"))
    return { kind: "custom", cron: details.default_schedule_cron };
  return { kind: "manual" };
}

/** Extract HH:MM from a five-field cron expression, falling back
 *  to 08:00 if the cron doesn't have plain integer fields. */
function extractDefaultTime(details: TemplateDetailsDto): string {
  const parts = details.default_schedule_cron.trim().split(/\s+/);
  if (parts.length < 2) return "08:00";
  const min = Number.parseInt(parts[0], 10);
  const hour = Number.parseInt(parts[1], 10);
  if (!Number.isFinite(min) || !Number.isFinite(hour)) return "08:00";
  return `${String(hour).padStart(2, "0")}:${String(min).padStart(2, "0")}`;
}
