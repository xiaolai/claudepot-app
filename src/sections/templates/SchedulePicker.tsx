import { useState } from "react";
import type {
  ScheduleDto,
  ScheduleShapeName,
  Weekday,
} from "../../types";

const WEEKDAY_LABELS: Record<Weekday, string> = {
  sun: "Sun",
  mon: "Mon",
  tue: "Tue",
  wed: "Wed",
  thu: "Thu",
  fri: "Fri",
  sat: "Sat",
};

interface Props {
  /** Allowed schedule shapes from the blueprint. */
  allowedShapes: ScheduleShapeName[];
  /** Default shape preselected. */
  defaultShape: ScheduleShapeName;
  /** Default time for shapes that take HH:MM. */
  defaultTime: string;
  /** Default cron string when the user picks Custom. */
  defaultCron: string;
  value: ScheduleDto;
  onChange: (next: ScheduleDto) => void;
}

/**
 * Semantic schedule picker. Hides cron entirely except behind the
 * "Custom (advanced)" escape hatch.
 *
 * The blueprint's `allowed_shapes` constrains which radios appear;
 * "manual" is treated as a real first-class option ("Only when I
 * run it"), not buried under advanced.
 */
export function SchedulePicker({
  allowedShapes,
  value,
  defaultTime,
  defaultCron,
  onChange,
}: Props) {
  const [time, setTime] = useState(currentTime(value, defaultTime));
  const [day, setDay] = useState<Weekday>(currentDay(value));
  const [hours, setHours] = useState(currentHours(value));
  const [cron, setCron] = useState(currentCron(value, defaultCron));

  function pick(kind: ScheduleShapeName) {
    switch (kind) {
      case "daily":
        onChange({ kind: "daily", time });
        break;
      case "weekdays":
        onChange({ kind: "weekdays", time });
        break;
      case "weekly":
        onChange({ kind: "weekly", day, time });
        break;
      case "hourly":
        onChange({ kind: "hourly", every_n_hours: hours });
        break;
      case "manual":
        onChange({ kind: "manual" });
        break;
      case "custom":
        onChange({ kind: "custom", cron });
        break;
    }
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-8)" }}>
      {allowedShapes.includes("daily") && (
        <Radio
          name="schedule"
          checked={value.kind === "daily"}
          onSelect={() => pick("daily")}
          label={
            <span>
              Each day at{" "}
              <TimeInput
                value={time}
                onChange={(v) => {
                  setTime(v);
                  if (value.kind === "daily") onChange({ kind: "daily", time: v });
                }}
              />
            </span>
          }
        />
      )}

      {allowedShapes.includes("weekdays") && (
        <Radio
          name="schedule"
          checked={value.kind === "weekdays"}
          onSelect={() => pick("weekdays")}
          label={
            <span>
              Each weekday at{" "}
              <TimeInput
                value={time}
                onChange={(v) => {
                  setTime(v);
                  if (value.kind === "weekdays")
                    onChange({ kind: "weekdays", time: v });
                }}
              />
            </span>
          }
        />
      )}

      {allowedShapes.includes("weekly") && (
        <Radio
          name="schedule"
          checked={value.kind === "weekly"}
          onSelect={() => pick("weekly")}
          label={
            <span>
              Each{" "}
              <select
                value={day}
                onChange={(e) => {
                  const d = e.target.value as Weekday;
                  setDay(d);
                  if (value.kind === "weekly")
                    onChange({ kind: "weekly", day: d, time });
                }}
                style={selectStyle}
              >
                {(Object.keys(WEEKDAY_LABELS) as Weekday[]).map((w) => (
                  <option key={w} value={w}>
                    {WEEKDAY_LABELS[w]}
                  </option>
                ))}
              </select>{" "}
              at{" "}
              <TimeInput
                value={time}
                onChange={(v) => {
                  setTime(v);
                  if (value.kind === "weekly")
                    onChange({ kind: "weekly", day, time: v });
                }}
              />
            </span>
          }
        />
      )}

      {allowedShapes.includes("hourly") && (
        <Radio
          name="schedule"
          checked={value.kind === "hourly"}
          onSelect={() => pick("hourly")}
          label={
            <span>
              Every{" "}
              <input
                type="number"
                min={1}
                max={23}
                value={hours}
                onChange={(e) => {
                  const n = Math.max(1, Math.min(23, +e.target.value || 1));
                  setHours(n);
                  if (value.kind === "hourly")
                    onChange({ kind: "hourly", every_n_hours: n });
                }}
                style={{ ...inputStyle, width: "var(--sp-32)" }}
              />{" "}
              hours
            </span>
          }
        />
      )}

      {allowedShapes.includes("manual") && (
        <Radio
          name="schedule"
          checked={value.kind === "manual"}
          onSelect={() => pick("manual")}
          label="Only when I run it"
        />
      )}

      {allowedShapes.includes("custom") && (
        <Radio
          name="schedule"
          checked={value.kind === "custom"}
          onSelect={() => pick("custom")}
          label={
            <span>
              Custom (advanced) —{" "}
              <input
                type="text"
                value={cron}
                placeholder="0 8 * * *"
                onChange={(e) => {
                  setCron(e.target.value);
                  if (value.kind === "custom")
                    onChange({ kind: "custom", cron: e.target.value });
                }}
                style={{ ...inputStyle, width: "16ch", fontFamily: "var(--font-mono)" }}
              />
            </span>
          }
        />
      )}
    </div>
  );
}

function Radio({
  name,
  checked,
  onSelect,
  label,
}: {
  name: string;
  checked: boolean;
  onSelect: () => void;
  label: React.ReactNode;
}) {
  return (
    <label
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-8)",
        cursor: "pointer",
        fontSize: "var(--fs-sm)",
        color: "var(--fg)",
      }}
    >
      <input
        type="radio"
        name={name}
        checked={checked}
        onChange={onSelect}
        style={{ accentColor: "var(--accent)" }}
      />
      <span>{label}</span>
    </label>
  );
}

function TimeInput({
  value,
  onChange,
}: {
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <input
      type="time"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      style={{ ...inputStyle, fontFamily: "var(--font-mono)" }}
    />
  );
}

const inputStyle: React.CSSProperties = {
  padding: "var(--sp-2) var(--sp-6)",
  border: "var(--bw-hair) solid var(--line)",
  borderRadius: "var(--r-1)",
  background: "var(--bg-raised)",
  color: "var(--fg)",
  fontSize: "var(--fs-sm)",
};

const selectStyle: React.CSSProperties = { ...inputStyle };

function currentTime(s: ScheduleDto, fallback: string): string {
  if (s.kind === "daily" || s.kind === "weekdays" || s.kind === "weekly") {
    return s.time;
  }
  return fallback;
}

function currentDay(s: ScheduleDto): Weekday {
  return s.kind === "weekly" ? s.day : "mon";
}

function currentHours(s: ScheduleDto): number {
  return s.kind === "hourly" ? s.every_n_hours : 4;
}

function currentCron(s: ScheduleDto, fallback: string): string {
  return s.kind === "custom" ? s.cron : fallback;
}
