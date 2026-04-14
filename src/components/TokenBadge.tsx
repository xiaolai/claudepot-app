export function TokenBadge({
  status,
  mins,
}: {
  status: string;
  mins: number | null;
}) {
  const kind = status.startsWith("valid")
    ? "ok"
    : status === "expired"
    ? "bad"
    : "warn";
  const label = kind === "ok" && mins != null ? `valid · ${mins}m` : status;
  const title =
    kind === "ok"
      ? `Access token valid, expires in ${mins ?? "?"} minutes. Auto-refreshes on switch.`
      : status === "expired"
      ? "Access token expired. Will refresh automatically on next switch."
      : status === "missing" || status === "no credentials"
      ? "No stored credentials. Log in to restore."
      : "Credential file is corrupt. Log in to restore.";
  return (
    <span className={`token-badge ${kind}`} title={title}>
      {label}
    </span>
  );
}
