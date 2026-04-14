export function ActivePill({
  label,
  email,
  disabled,
  disabledHint,
}: {
  label: string;
  email: string | null;
  disabled?: boolean;
  disabledHint?: string;
}) {
  if (disabled) {
    return (
      <div className="pill disabled" title={disabledHint}>
        <span className="pill-label">{label}</span>
        <span className="pill-value muted">{disabledHint}</span>
      </div>
    );
  }
  return (
    <div className={`pill ${email ? "active" : ""}`}>
      <span className="pill-label">{label}</span>
      <span className="pill-value">{email ?? "—"}</span>
    </div>
  );
}
