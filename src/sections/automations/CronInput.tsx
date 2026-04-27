import { useEffect, useRef, useState } from "react";
import { api } from "../../api";

interface CronInputProps {
  value: string;
  onChange: (next: string) => void;
  /** Notify parent of validity so they can gate submit. */
  onValidityChange?: (valid: boolean) => void;
  disabled?: boolean;
}

/**
 * Cron expression input with live "Next 5 fire times" preview.
 * Debounces validation by 250ms so we don't hammer the IPC for
 * every keystroke.
 */
export function CronInput({
  value,
  onChange,
  onValidityChange,
  disabled,
}: CronInputProps) {
  const [valid, setValid] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [nextRuns, setNextRuns] = useState<string[]>([]);
  const debounceRef = useRef<number | undefined>(undefined);

  useEffect(() => {
    if (debounceRef.current !== undefined) {
      window.clearTimeout(debounceRef.current);
    }
    debounceRef.current = window.setTimeout(async () => {
      try {
        const result = await api.automationsValidateCron(value);
        setValid(result.valid);
        setError(result.error);
        setNextRuns(result.next_runs);
        onValidityChange?.(result.valid);
      } catch (e) {
        setValid(false);
        setError(String(e));
        setNextRuns([]);
        onValidityChange?.(false);
      }
    }, 250);
    return () => {
      if (debounceRef.current !== undefined) {
        window.clearTimeout(debounceRef.current);
      }
    };
  }, [value, onValidityChange]);

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-6)",
      }}
    >
      <label
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "var(--sp-4)",
          fontSize: "var(--fs-xs)",
          color: "var(--fg-2)",
        }}
      >
        <span>Cron expression (5 fields: min hour dom mon dow)</span>
        <input
          type="text"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          disabled={disabled}
          placeholder="0 9 * * *"
          spellCheck={false}
          style={{
            fontFamily: "var(--ff-mono)",
            fontSize: "var(--fs-sm)",
            padding: "var(--sp-6) var(--sp-8)",
            border: `var(--bw-hair) solid ${
              valid ? "var(--line)" : "var(--danger)"
            }`,
            borderRadius: "var(--r-2)",
            background: "var(--bg-raised)",
            color: "var(--fg)",
          }}
        />
      </label>
      {error && (
        <div
          style={{
            fontSize: "var(--fs-2xs)",
            color: "var(--danger)",
          }}
        >
          {error}
        </div>
      )}
      {valid && nextRuns.length > 0 && (
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: "var(--sp-2)",
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-2)",
            fontFamily: "var(--ff-mono)",
          }}
        >
          <span style={{ color: "var(--fg-3)" }}>Next 5 runs (UTC):</span>
          {nextRuns.map((iso) => (
            <span key={iso}>{formatIso(iso)}</span>
          ))}
        </div>
      )}
    </div>
  );
}

function formatIso(iso: string): string {
  // 2026-04-28T09:00:00+00:00 → 2026-04-28 09:00 UTC
  const m = /^(\d{4}-\d{2}-\d{2})T(\d{2}:\d{2})/.exec(iso);
  return m ? `${m[1]} ${m[2]} UTC` : iso;
}
