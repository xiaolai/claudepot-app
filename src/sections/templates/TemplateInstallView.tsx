import { useEffect, useState } from "react";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
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
  templateId: string;
  /** Cancel button — closes the install view (e.g. back to
   *  gallery or close the modal). */
  onBack: () => void;
  onInstalled: () => void;
  onError: (msg: string) => void;
  onOpenThirdParties: () => void;
  /** Optional label for the back/cancel button. Defaults to
   *  "Cancel" — when rendered inside the gallery it's overridden
   *  to "Back" so the user knows they're returning to the grid. */
  backLabel?: string;
}

/**
 * Pure content view for the template-install flow. Caller
 * provides the surrounding modal. Layout is:
 *
 *   [ header — fixed at top ]
 *   [ scrollable body ]
 *   [ sticky action bar — fixed at bottom, with bottom padding ]
 *
 * The body grows to fill the available height; if its content
 * overflows, only the body scrolls, so the action bar stays
 * visible. This fixes the prior "buttons clipped at the bottom"
 * issue when a description + sample report + scope statements
 * exceeded the modal's max height.
 */
export function TemplateInstallView({
  templateId,
  onBack,
  onInstalled,
  onError,
  onOpenThirdParties,
  backLabel = "Cancel",
}: Props) {
  const [details, setDetails] = useState<TemplateDetailsDto | null>(null);
  const [routes, setRoutes] = useState<TemplateRouteSummaryDto[]>([]);
  const [routeId, setRouteId] = useState<string | null>(null);
  const [schedule, setSchedule] = useState<ScheduleDto | null>(null);
  const [sampleOpen, setSampleOpen] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
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
        if (cancelled) return;
        setDetails(d);
        setRoutes(rs);
        setSchedule(initialSchedule(d.allowed_schedule_shapes, d));
      })
      .catch((e: unknown) => {
        if (!cancelled) onError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [templateId, onError]);

  const ready = details !== null && schedule !== null;

  async function handleInstall() {
    if (!details || !schedule) return;
    setBusy(true);
    const instance: TemplateInstanceDto = {
      blueprint_id: details.summary.id,
      blueprint_schema_version: details.schema_version,
      placeholder_values: {},
      route_id: routeId ?? undefined,
      schedule,
    };
    try {
      await api.templatesInstall(instance);
      onInstalled();
    } catch (e: unknown) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  }

  if (!ready) {
    return (
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          padding: "var(--sp-32)",
          color: "var(--fg-faint)",
          fontSize: "var(--fs-sm)",
          minHeight: "tokens.banner.min.width",
        }}
      >
        Loading template…
      </div>
    );
  }

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        // The modal itself caps height; this view fills it.
        minHeight: 0,
        flex: 1,
      }}
    >
      {/* Header — fixed at top */}
      <div
        style={{
          padding: "var(--sp-16) var(--sp-20) var(--sp-12)",
          borderBottom: "var(--bw-hair) solid var(--line)",
          flexShrink: 0,
        }}
      >
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

      {/* Scrollable body */}
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "var(--sp-16)",
          padding: "var(--sp-16) var(--sp-20)",
          overflowY: "auto",
          flex: 1,
          minHeight: 0,
        }}
      >
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
          <p
            style={{
              margin: 0,
              fontSize: "var(--fs-xs)",
              color: "var(--fg-faint)",
            }}
          >
            Sends data to your selected route.
          </p>
        )}
      </div>

      {/* Sticky action bar — always visible, generous bottom padding */}
      <div
        style={{
          display: "flex",
          justifyContent: "flex-end",
          gap: "var(--sp-8)",
          padding: "var(--sp-12) var(--sp-20) var(--sp-16)",
          borderTop: "var(--bw-hair) solid var(--line)",
          background: "var(--bg)",
          flexShrink: 0,
        }}
      >
        <Button variant="ghost" onClick={onBack} disabled={busy}>
          {backLabel}
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
  );
}

function installDisabled(
  details: TemplateDetailsDto,
  routes: TemplateRouteSummaryDto[],
): boolean {
  if (
    details.summary.privacy === "local" &&
    !routes.some((r) => r.is_capable && r.is_local)
  ) {
    return true;
  }
  return false;
}

function pickInitialShape(shapes: ScheduleShapeName[]): ScheduleShapeName {
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

function extractDefaultTime(details: TemplateDetailsDto): string {
  const parts = details.default_schedule_cron.trim().split(/\s+/);
  if (parts.length < 2) return "08:00";
  const min = Number.parseInt(parts[0], 10);
  const hour = Number.parseInt(parts[1], 10);
  if (!Number.isFinite(min) || !Number.isFinite(hour)) return "08:00";
  return `${String(hour).padStart(2, "0")}:${String(min).padStart(2, "0")}`;
}
