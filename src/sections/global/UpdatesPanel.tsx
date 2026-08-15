import { useCallback, useEffect, useMemo, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import { api } from "../../api";
import { i18n } from "../../lib/i18n";
import { renderError } from "../../lib/i18n-error";
import type {
  AutoInstallOutcome,
  CliInstall,
  DesktopInstall,
  UpdatesStatusDto,
} from "../../types/updates";
import { Button } from "../../components/primitives/Button";
import { Glyph } from "../../components/primitives/Glyph";
import { SectionLabel } from "../../components/primitives/SectionLabel";
import { Tag } from "../../components/primitives/Tag";
import { NF } from "../../icons";

type Comparison = "older" | "equal" | "newer" | "unknown";

function compareVersions(installed: string | null, latest: string | null): Comparison {
  if (!installed || !latest) return "unknown";
  const parse = (s: string) =>
    s.split(/[.-]/).map((p) => Number.parseInt(p, 10) || 0);
  const a = parse(installed);
  const b = parse(latest);
  const len = Math.max(a.length, b.length);
  for (let i = 0; i < len; i++) {
    const av = a[i] ?? 0;
    const bv = b[i] ?? 0;
    if (av < bv) return "older";
    if (av > bv) return "newer";
  }
  return "equal";
}

/**
 * Build a truthful confirmation string for a manual install. The user
 * can click "Update now" or "Reinstall" — both invoke the same Tauri
 * command, so we infer what actually happened from the version diff
 * instead of trusting the button label. `after === null` means the
 * subprocess reported success but the post-install version probe
 * couldn't confirm a number.
 */
function describeInstallOutcome(
  surface: string,
  before: string | null,
  after: string | null,
): string {
  if (after && before === after) {
    return i18n.t("updates.outcome.reinstalled", {
      ns: "global",
      surface,
      version: after,
    });
  }
  if (after && before) {
    return i18n.t("updates.outcome.updated", {
      ns: "global",
      surface,
      before,
      after,
    });
  }
  if (after) {
    return i18n.t("updates.outcome.installed", {
      ns: "global",
      surface,
      version: after,
    });
  }
  return i18n.t("updates.outcome.completed", { ns: "global", surface });
}

function formatRelativeTime(unix: number | null): string {
  if (!unix) return i18n.t("updates.time.never", { ns: "global" });
  const now = Date.now() / 1000;
  const dt = now - unix;
  if (dt < 60) return i18n.t("updates.time.justNow", { ns: "global" });
  if (dt < 3600) {
    return i18n.t("updates.time.minAgo", { ns: "global", n: Math.round(dt / 60) });
  }
  if (dt < 86400) {
    return i18n.t("updates.time.hAgo", { ns: "global", n: Math.round(dt / 3600) });
  }
  return i18n.t("updates.time.dAgo", { ns: "global", n: Math.round(dt / 86400) });
}

function StatusBadge({ comparison }: { comparison: Comparison }) {
  const { t } = useTranslation("global");
  switch (comparison) {
    case "older":
      return (
        <Tag tone="warn">
          <Glyph g={NF.download} />
          {t("updates.badge.updateAvailable")}
        </Tag>
      );
    case "equal":
      return (
        <Tag tone="ok">
          <Glyph g={NF.check} />
          {t("updates.badge.upToDate")}
        </Tag>
      );
    case "newer":
      return <Tag tone="neutral">{t("updates.badge.newerThanChannel")}</Tag>;
    default:
      return <Tag tone="neutral">{t("updates.badge.unknown")}</Tag>;
  }
}

/**
 * The only affordance Updates offers for the two update flags: a way to
 * reach the surface that owns them. A blocker the user is told about and
 * cannot act on is a dead end, which is what this pane was before.
 */
function EditEnvVarsLink({ onClick }: { onClick: () => void }) {
  const { t } = useTranslation("global");
  return (
    <button
      type="button"
      onClick={onClick}
      style={{
        background: "transparent",
        border: "none",
        padding: 0,
        marginLeft: "var(--sp-6)",
        color: "var(--accent)",
        textDecoration: "underline",
        font: "inherit",
      }}
    >
      {t("updates.editEnvVars")}
    </button>
  );
}

function CliCard({
  status,
  busy,
  onCheck,
  onInstall,
  onChannelSet,
  onAutoToggle,
  onEditEnvVars,
}: {
  status: UpdatesStatusDto;
  busy: boolean;
  onCheck: () => void;
  onInstall: () => void;
  onChannelSet: (channel: "latest" | "stable") => void;
  onAutoToggle: (v: boolean) => void;
  /** Deep-link to Config → Env Variables, which owns these two flags.
   *  Updates only *reports* them — nothing here has ever written them,
   *  so duplicating a control would manufacture an ownership conflict
   *  rather than resolve one. */
  onEditEnvVars: () => void;
}) {
  const { t } = useTranslation("global");
  const cli = status.cli;
  const active = cli.installs.find((i) => i.is_active);
  const installed = active?.version ?? null;
  const latest = cli.latest_remote ?? cli.last_known;
  const comparison = compareVersions(installed, latest);

  const disabledReason = cli.cc_settings.disable_updates
    ? t("updates.cli.blockedInSettings")
    : !active
      ? t("updates.cli.noActiveBinary")
      : null;

  return (
    <Card>
      {/* "CC CLI" is the product surface's own name — identical in
          every locale, so it stays a literal rather than a key whose
          translations would all read the same. */}
      <CardHeader
        title="CC CLI"
        subtitle={
          installed
            ? t("updates.installedVersion", { version: installed })
            : t("updates.notInstalled")
        }
        badge={installed ? <StatusBadge comparison={comparison} /> : undefined}
      />
      <Row label={t("updates.row.latest")}>
        {latest ?? (
          <em style={{ color: "var(--fg-faint)" }}>
            {t("updates.networkProbeFailed")}
          </em>
        )}
        {cli.last_check_unix && (
          <Sub>
            {t("updates.checkedRelative", {
              relative: formatRelativeTime(cli.last_check_unix),
            })}
          </Sub>
        )}
      </Row>
      <Row label={t("updates.row.channel")}>
        <ChannelToggle
          value={cli.channel}
          onChange={onChannelSet}
          minimumVersion={cli.cc_settings.minimum_version}
        />
      </Row>
      <Row label={t("updates.row.autoUpdate")}>
        <ToggleWithLabel
          on={status.settings.cli.force_update_on_check}
          onChange={onAutoToggle}
          disabled={cli.cc_settings.disable_updates}
          label={
            cli.cc_settings.disable_updates
              ? t("updates.cli.blockedByFlag")
              : t("updates.cli.autoUpdateHint")
          }
        />
      </Row>
      {cli.cc_settings.minimum_version && (
        <Row label={t("updates.row.floor")}>
          {/* CC settings key + its raw value — not prose. */}
          <code style={{ fontSize: "var(--fs-xs)" }}>
            minimumVersion = {cli.cc_settings.minimum_version}
          </code>
        </Row>
      )}
      {cli.cc_settings.disable_autoupdater && (
        <Warning>
          <Glyph g={NF.warn} />
          {t("updates.cli.warnAutoupdaterOff")}
          <EditEnvVarsLink onClick={onEditEnvVars} />
        </Warning>
      )}
      {cli.cc_settings.disable_updates && (
        <Warning>
          <Glyph g={NF.warn} />
          {t("updates.cli.warnUpdatesBlocked")}
          <EditEnvVarsLink onClick={onEditEnvVars} />
        </Warning>
      )}
      {cli.running_count > 0 && (
        <Row label={t("updates.row.running")}>
          {t("updates.cli.runningProcesses", { count: cli.running_count })}
        </Row>
      )}
      {cli.installs.length > 1 && (
        <Row label={t("updates.row.installs")}>
          <ul style={{ margin: 0, paddingLeft: "var(--sp-16)", listStyle: "none" }}>
            {cli.installs.map((i) => (
              <InstallRow key={i.binary_path} install={i} />
            ))}
          </ul>
        </Row>
      )}
      <Actions
        primary={{
          label:
            comparison === "older"
              ? t("updates.action.updateNow")
              : t("updates.action.reinstall"),
          onClick: onInstall,
          disabled: busy || disabledReason !== null,
          glyph: NF.download,
          variant: comparison === "older" ? "solid" : "outline",
        }}
        secondary={{
          label: t("updates.action.checkNow"),
          onClick: onCheck,
          disabled: busy,
          glyph: NF.refresh,
        }}
        disabledReason={disabledReason}
      />
    </Card>
  );
}

function DesktopCard({
  status,
  busy,
  onCheck,
  onInstall,
  onAutoToggle,
}: {
  status: UpdatesStatusDto;
  busy: boolean;
  onCheck: () => void;
  onInstall: () => void;
  onAutoToggle: (v: boolean) => void;
}) {
  const { t } = useTranslation("global");
  const ds = status.desktop;
  const installed = ds.install?.version ?? null;
  const latest = ds.latest_remote;
  const comparison = compareVersions(installed, latest);

  const noInstall = !ds.install;
  const notManageable = ds.install && !ds.install.manageable;
  const disabledReason = noInstall
    ? t("updates.desktop.noInstall")
    : notManageable
      ? t("updates.desktop.notManageable", { source: ds.install!.source })
      : ds.running
        ? t("updates.desktop.isRunning")
        : null;

  return (
    <Card>
      {/* Product name — same in every locale (see CliCard). */}
      <CardHeader
        title="Claude Desktop"
        subtitle={
          installed
            ? t("updates.installedVersion", { version: installed })
            : t("updates.notInstalled")
        }
        badge={installed ? <StatusBadge comparison={comparison} /> : undefined}
      />
      {ds.install && (
        <Row label={t("updates.row.source")}>
          <SourceTag source={ds.install.source} />
        </Row>
      )}
      <Row label={t("updates.row.latest")}>
        {latest ?? (
          <em style={{ color: "var(--fg-faint)" }}>
            {t("updates.networkProbeFailed")}
          </em>
        )}
        {ds.last_check_unix && (
          <Sub>
            {t("updates.checkedRelative", {
              relative: formatRelativeTime(ds.last_check_unix),
            })}
          </Sub>
        )}
      </Row>
      <Row label={t("updates.row.status")}>
        <Tag tone={ds.running ? "warn" : "neutral"}>
          {ds.running
            ? t("updates.desktop.statusRunning")
            : t("updates.desktop.statusNotRunning")}
        </Tag>
      </Row>
      <Row label={t("updates.row.autoInstall")}>
        <ToggleWithLabel
          on={status.settings.desktop.auto_install_when_quit}
          onChange={onAutoToggle}
          label={t("updates.desktop.autoInstallHint")}
        />
      </Row>
      <Actions
        primary={{
          label:
            comparison === "older"
              ? t("updates.action.updateNow")
              : t("updates.action.reinstall"),
          onClick: onInstall,
          disabled: busy || disabledReason !== null,
          glyph: NF.download,
          variant: comparison === "older" ? "solid" : "outline",
        }}
        secondary={{
          label: t("updates.action.checkNow"),
          onClick: onCheck,
          disabled: busy,
          glyph: NF.refresh,
        }}
        disabledReason={disabledReason}
      />
    </Card>
  );
}

function SettingsCard({
  status,
  busy,
  onCliNotifyToggle,
  onCliNotifyOsToggle,
  onDesktopNotifyToggle,
  onDesktopNotifyOsToggle,
  onMinimumVersionClear,
}: {
  status: UpdatesStatusDto;
  busy: boolean;
  onCliNotifyToggle: (v: boolean) => void;
  onCliNotifyOsToggle: (v: boolean) => void;
  onDesktopNotifyToggle: (v: boolean) => void;
  onDesktopNotifyOsToggle: (v: boolean) => void;
  onMinimumVersionClear: () => void;
}) {
  const { t } = useTranslation("global");
  return (
    <Card>
      <CardHeader
        title={t("updates.settings.title")}
        subtitle={t("updates.settings.subtitle")}
      />
      <Row label={t("updates.row.cliNotify")}>
        <span style={{ display: "flex", flexDirection: "column", gap: "var(--sp-6)" }}>
          <ToggleWithLabel
            on={status.settings.cli.notify_on_available}
            onChange={onCliNotifyToggle}
            label={t("updates.settings.trayBadgeHint")}
          />
          <ToggleWithLabel
            on={status.settings.cli.notify_os_on_available}
            onChange={onCliNotifyOsToggle}
            label={t("updates.settings.osNotifyHint")}
          />
        </span>
      </Row>
      <Row label={t("updates.row.desktopNotify")}>
        <span style={{ display: "flex", flexDirection: "column", gap: "var(--sp-6)" }}>
          <ToggleWithLabel
            on={status.settings.desktop.notify_on_available}
            onChange={onDesktopNotifyToggle}
            label={t("updates.settings.trayBadgeHint")}
          />
          <ToggleWithLabel
            on={status.settings.desktop.notify_os_on_available}
            onChange={onDesktopNotifyOsToggle}
            label={t("updates.settings.osNotifyHint")}
          />
        </span>
      </Row>
      {status.cli.cc_settings.minimum_version && (
        <Row label={t("updates.row.pin")}>
          <span style={{ display: "inline-flex", alignItems: "center", gap: "var(--sp-8)" }}>
            {/* CC settings key + its raw value — not prose. */}
            <code style={{ fontSize: "var(--fs-xs)" }}>
              minimumVersion = {status.cli.cc_settings.minimum_version}
            </code>
            <Button
              size="sm"
              variant="outline"
              onClick={onMinimumVersionClear}
              disabled={busy}
            >
              {t("updates.settings.clear")}
            </Button>
          </span>
        </Row>
      )}
      <Row label={t("updates.row.source")}>
        <span style={{ fontSize: "var(--fs-xs)", color: "var(--fg-faint)" }}>
          <Trans
            ns="global"
            i18nKey="updates.settings.sourceNote"
            components={{ code: <code /> }}
          />
        </span>
      </Row>
    </Card>
  );
}

// ─── Inline primitives (kept local — shape isn't reusable elsewhere) ──

function Card({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        background: "var(--bg)",
        padding: "var(--sp-14) var(--sp-16)",
        marginBottom: "var(--sp-16)",
      }}
    >
      {children}
    </div>
  );
}

function CardHeader({
  title,
  subtitle,
  badge,
}: {
  title: string;
  subtitle: string;
  badge?: React.ReactNode;
}) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "baseline",
        justifyContent: "space-between",
        marginBottom: "var(--sp-12)",
      }}
    >
      <div>
        <div style={{ fontSize: "var(--fs-base)", fontWeight: 600 }}>{title}</div>
        <div style={{ fontSize: "var(--fs-xs)", color: "var(--fg-muted)" }}>
          {subtitle}
        </div>
      </div>
      {badge}
    </div>
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div
      style={{
        display: "flex",
        gap: "var(--sp-12)",
        padding: "var(--sp-4) 0",
        fontSize: "var(--fs-sm)",
      }}
    >
      <div
        style={{
          width: "var(--sp-96)",
          flexShrink: 0,
          color: "var(--fg-faint)",
          fontSize: "var(--fs-xs)",
          paddingTop: "var(--sp-2)",
        }}
      >
        {label}
      </div>
      <div style={{ flex: 1, display: "flex", alignItems: "center", flexWrap: "wrap", gap: "var(--sp-8)" }}>
        {children}
      </div>
    </div>
  );
}

function Sub({ children }: { children: React.ReactNode }) {
  return (
    <span
      style={{ fontSize: "var(--fs-xs)", color: "var(--fg-faint)", marginLeft: "var(--sp-8)" }}
    >
      ({children})
    </span>
  );
}

function Warning({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        marginTop: "var(--sp-8)",
        padding: "var(--sp-6) var(--sp-10)",
        background: "color-mix(in oklch, var(--danger) 8%, transparent)",
        borderLeft: "var(--bw-hair) solid var(--danger)",
        borderRadius: "var(--r-1)",
        fontSize: "var(--fs-xs)",
        color: "var(--fg-muted)",
        display: "inline-flex",
        alignItems: "center",
        gap: "var(--sp-6)",
      }}
    >
      {children}
    </div>
  );
}

/**
 * Pill-switch toggle, mirroring the one in `SettingsSection.tsx`.
 * Native `<input type="checkbox">` rendering inside the Tauri webview
 * looks out of place against paper-mono and proved unreliable to
 * click in some themes. The button + role="switch" pattern is the
 * project's canonical toggle. The description text is rendered as a
 * sibling by the caller (kept out of the toggle's API to match the
 * existing Toggle exactly — same shape, same a11y semantics).
 */
function Toggle({
  on,
  onChange,
  disabled,
}: {
  on: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      disabled={disabled}
      onClick={() => !disabled && onChange(!on)}
      className="pm-focus"
      style={{
        width: "var(--toggle-track-w)",
        height: "var(--toggle-track-h)",
        borderRadius: "var(--r-pill)",
        background: on ? "var(--accent)" : "var(--bg-active)",
        border: `var(--bw-hair) solid ${on ? "var(--accent)" : "var(--line-strong)"}`,
        position: "relative",
        opacity: disabled ? "var(--opacity-disabled)" : 1,
        transition: "background var(--dur-base) var(--ease-linear)",
        flexShrink: 0,
        padding: 0,
      }}
    >
      <span
        aria-hidden
        style={{
          position: "absolute",
          top: "var(--toggle-thumb-off)",
          left: on ? "var(--toggle-thumb-on)" : "var(--toggle-thumb-off)",
          width: "var(--toggle-thumb-d)",
          height: "var(--toggle-thumb-d)",
          borderRadius: "var(--r-pill)",
          background: "var(--bg-raised)",
          boxShadow: "var(--shadow-thumb)",
          transition: "left var(--dur-base) var(--ease-linear)",
        }}
      />
    </button>
  );
}

/** Toggle + label + optional description, in one row of content. */
function ToggleWithLabel({
  on,
  onChange,
  disabled,
  label,
}: {
  on: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  label: string;
}) {
  return (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: "var(--sp-10)",
      }}
    >
      <Toggle on={on} onChange={onChange} disabled={disabled} />
      <span
        style={{
          fontSize: "var(--fs-sm)",
          color: disabled ? "var(--fg-faint)" : "var(--fg)",
        }}
      >
        {label}
      </span>
    </span>
  );
}

function ChannelToggle({
  value,
  onChange,
  minimumVersion,
}: {
  value: string;
  onChange: (v: "latest" | "stable") => void;
  minimumVersion: string | null;
}) {
  const { t } = useTranslation("global");
  return (
    <div style={{ display: "inline-flex", gap: "var(--sp-4)" }}>
      {/* "latest" / "stable" are CC's own `autoUpdatesChannel` wire
          values, rendered raw so the button reads as the value it
          writes. */}
      <Button
        size="sm"
        variant={value === "latest" ? "subtle" : "ghost"}
        active={value === "latest"}
        onClick={() => onChange("latest")}
      >
        latest
      </Button>
      <Button
        size="sm"
        variant={value === "stable" ? "subtle" : "ghost"}
        active={value === "stable"}
        onClick={() => onChange("stable")}
        title={
          minimumVersion
            ? t("updates.channel.pinHonored", { version: minimumVersion })
            : undefined
        }
      >
        stable
      </Button>
    </div>
  );
}

function SourceTag({ source }: { source: DesktopInstall["source"] }) {
  const { t } = useTranslation("global");
  const labels: Record<DesktopInstall["source"], string> = {
    homebrew: t("updates.source.homebrew"),
    "direct-dmg": t("updates.source.directDmg"),
    setapp: t("updates.source.setapp"),
    "mac-app-store": t("updates.source.macAppStore"),
    "user-local": t("updates.source.userLocal"),
  };
  return <Tag tone="neutral">{labels[source]}</Tag>;
}

function InstallRow({ install }: { install: CliInstall }) {
  const { t } = useTranslation("global");
  const kindLabel: Record<CliInstall["kind"], string> = {
    "native-curl": t("updates.installKind.nativeCurl"),
    "npm-global": "npm",
    "homebrew-stable": "homebrew (stable)",
    "homebrew-latest": "homebrew (latest)",
    apt: "apt",
    dnf: "dnf",
    apk: "apk",
    "win-get": "winget",
    unknown: t("updates.installKind.unknown"),
  };
  return (
    <li
      style={{
        padding: "var(--sp-2) 0",
        fontSize: "var(--fs-xs)",
        color: install.is_active ? "var(--fg)" : "var(--fg-muted)",
        display: "flex",
        gap: "var(--sp-8)",
        alignItems: "center",
      }}
    >
      {install.is_active && (
        <Tag tone="ok">
          <Glyph g={NF.dot} />
          {t("updates.installActive")}
        </Tag>
      )}
      <code>{install.binary_path}</code>
      <span>{install.version ?? "?"}</span>
      <span style={{ color: "var(--fg-faint)" }}>({kindLabel[install.kind]})</span>
    </li>
  );
}

/**
 * One-line banner summarising what happened on the most recent
 * auto-install attempt for a given surface. Hidden when the toggle
 * is off (`disabled`) or nothing was needed (`up-to-date`) — those
 * cases would just be noise.
 */
function AutoOutcomeBanner({
  outcome,
  surface,
}: {
  outcome: AutoInstallOutcome;
  surface: string;
}) {
  const { t } = useTranslation("global");
  if (outcome.kind === "disabled" || outcome.kind === "up-to-date") {
    return null;
  }
  let tone: "ok" | "warn" | "error";
  let text: string;
  switch (outcome.kind) {
    case "installed":
      tone = "ok";
      text = outcome.version
        ? t("updates.auto.installedVersion", {
            surface,
            version: outcome.version,
          })
        : t("updates.auto.installed", { surface });
      break;
    case "skipped":
      tone = "warn";
      text = t("updates.auto.skipped", { surface, reason: outcome.reason });
      break;
    case "failed":
      tone = "error";
      text = t("updates.auto.failed", { surface, error: outcome.error });
      break;
  }
  const color =
    tone === "error"
      ? "var(--danger)"
      : tone === "warn"
        ? "var(--warn)"
        : "var(--accent)";
  return (
    <div
      style={{
        margin: "var(--sp-8) 0",
        padding: "var(--sp-6) var(--sp-12)",
        borderRadius: "var(--r-2)",
        fontSize: "var(--fs-xs)",
        background: `color-mix(in oklch, ${color} 8%, transparent)`,
        borderLeft: `var(--bw-hair) solid ${color}`,
        color: "var(--fg-muted)",
      }}
    >
      {text}
    </div>
  );
}

function Actions({
  primary,
  secondary,
  disabledReason,
}: {
  primary: {
    label: string;
    onClick: () => void;
    disabled: boolean;
    glyph: typeof NF.download;
    variant: "solid" | "outline";
  };
  secondary: {
    label: string;
    onClick: () => void;
    disabled: boolean;
    glyph: typeof NF.refresh;
  };
  disabledReason: string | null;
}) {
  return (
    <div
      style={{
        marginTop: "var(--sp-12)",
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-8)",
        flexWrap: "wrap",
      }}
    >
      <Button
        variant={primary.variant}
        size="sm"
        glyph={primary.glyph}
        disabled={primary.disabled}
        onClick={primary.onClick}
      >
        {primary.label}
      </Button>
      <Button
        variant="ghost"
        size="sm"
        glyph={secondary.glyph}
        disabled={secondary.disabled}
        onClick={secondary.onClick}
      >
        {secondary.label}
      </Button>
      {disabledReason && (
        <span style={{ fontSize: "var(--fs-xs)", color: "var(--fg-faint)" }}>
          {disabledReason}
        </span>
      )}
    </div>
  );
}

// ─── The panel itself ─────────────────────────────────────────────────

export function UpdatesPanel({
  onEditEnvVars,
}: {
  /** Optional so the panel still renders standalone (and in tests). */
  onEditEnvVars?: () => void;
} = {}) {
  const { t } = useTranslation("global");
  const [status, setStatus] = useState<UpdatesStatusDto | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);

  const refresh = useCallback(async (forceProbe: boolean) => {
    setBusy(true);
    setError(null);
    try {
      const next = forceProbe
        ? await api.updatesCheckNow()
        : await api.updatesStatusGet();
      setStatus(next);
    } catch (e: unknown) {
      setError(renderError(e));
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    void refresh(false);
  }, [refresh]);

  // Subscribe to backend cycle events so the panel reflects what the
  // background poller did without the user having to click anything.
  // The emit happens once per cycle (default cadence: 4 h, but the
  // user can pin the toggle and click "Check now" to rerun on demand).
  useTauriEvent("updates::cycle-complete", () => {
    void refresh(false);
  });

  const onChannelSet = useCallback(
    async (channel: "latest" | "stable") => {
      setBusy(true);
      setError(null);
      try {
        await api.updatesChannelSet(channel);
        setInfo(t("updates.info.wroteChannel", { channel }));
        await refresh(false);
      } catch (e: unknown) {
        setError(renderError(e));
      } finally {
        setBusy(false);
      }
    },
    [refresh, t],
  );

  const onCliInstall = useCallback(async () => {
    setBusy(true);
    setError(null);
    setInfo(null);
    const before =
      status?.cli.installs.find((i) => i.is_active)?.version ?? null;
    try {
      const res = await api.updatesCliInstall();
      setInfo(describeInstallOutcome("CC CLI", before, res.installed_after));
      await refresh(true);
    } catch (e: unknown) {
      setError(renderError(e));
    } finally {
      setBusy(false);
    }
  }, [refresh, status]);

  const onDesktopInstall = useCallback(async () => {
    setBusy(true);
    setError(null);
    setInfo(null);
    const before = status?.desktop.install?.version ?? null;
    try {
      const res = await api.updatesDesktopInstall();
      setInfo(
        describeInstallOutcome("Claude Desktop", before, res.version_after),
      );
      await refresh(true);
    } catch (e: unknown) {
      setError(renderError(e));
    } finally {
      setBusy(false);
    }
  }, [refresh, status]);

  const setSettingsField = useCallback(
    async (
      patch: Parameters<typeof api.updatesSettingsSet>[0],
      label: string,
    ) => {
      setBusy(true);
      setError(null);
      try {
        await api.updatesSettingsSet(patch);
        setInfo(label);
        await refresh(false);
      } catch (e: unknown) {
        setError(renderError(e));
      } finally {
        setBusy(false);
      }
    },
    [refresh],
  );

  const onCliNotifyToggle = useCallback(
    (v: boolean) => setSettingsField({ cliNotifyOnAvailable: v }, t("updates.info.saved")),
    [setSettingsField, t],
  );
  const onCliNotifyOsToggle = useCallback(
    (v: boolean) => setSettingsField({ cliNotifyOsOnAvailable: v }, t("updates.info.saved")),
    [setSettingsField, t],
  );
  const onDesktopNotifyToggle = useCallback(
    (v: boolean) => setSettingsField({ desktopNotifyOnAvailable: v }, t("updates.info.saved")),
    [setSettingsField, t],
  );
  const onDesktopNotifyOsToggle = useCallback(
    (v: boolean) =>
      setSettingsField({ desktopNotifyOsOnAvailable: v }, t("updates.info.saved")),
    [setSettingsField, t],
  );
  const onCliAutoToggle = useCallback(
    (v: boolean) => setSettingsField({ cliForceUpdateOnCheck: v }, t("updates.info.saved")),
    [setSettingsField, t],
  );
  const onDesktopAutoToggle = useCallback(
    (v: boolean) =>
      setSettingsField({ desktopAutoInstallWhenQuit: v }, t("updates.info.saved")),
    [setSettingsField, t],
  );
  const onMinimumVersionClear = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      await api.updatesMinimumVersionSet(null);
      setInfo(t("updates.info.clearedMinimumVersion"));
      await refresh(false);
    } catch (e: unknown) {
      setError(renderError(e));
    } finally {
      setBusy(false);
    }
  }, [refresh, t]);

  const onCheckNow = useCallback(() => refresh(true), [refresh]);

  const banner = useMemo(() => {
    if (error) return { tone: "error" as const, text: error };
    if (info) return { tone: "ok" as const, text: info };
    return null;
  }, [error, info]);

  if (!status) {
    return (
      <div style={{ padding: "var(--sp-16)", color: "var(--fg-faint)" }}>
        {t("updates.loading")}
      </div>
    );
  }

  return (
    <div style={{ padding: "var(--sp-16)", maxWidth: "var(--content-cap-lg)" }}>
      <SectionLabel
        right={
          <Button
            size="sm"
            variant="ghost"
            glyph={NF.refresh}
            onClick={onCheckNow}
            disabled={busy}
          >
            {t("updates.action.checkNow")}
          </Button>
        }
      >
        {t("updates.title")}
      </SectionLabel>

      {banner && (
        <div
          style={{
            margin: "var(--sp-8) 0 var(--sp-12)",
            padding: "var(--sp-8) var(--sp-12)",
            borderRadius: "var(--r-2)",
            fontSize: "var(--fs-sm)",
            background:
              banner.tone === "error"
                ? "color-mix(in oklch, var(--danger) 10%, transparent)"
                : "color-mix(in oklch, var(--accent) 10%, transparent)",
            color: banner.tone === "error" ? "var(--danger)" : "var(--fg)",
            borderLeft: `var(--bw-hair) solid ${banner.tone === "error" ? "var(--danger)" : "var(--accent)"}`,
          }}
        >
          {banner.text}
        </div>
      )}

      <AutoOutcomeBanner outcome={status.cli_auto_outcome} surface="CLI" />
      <AutoOutcomeBanner outcome={status.desktop_auto_outcome} surface="Desktop" />
      <CliCard
        status={status}
        busy={busy}
        onCheck={onCheckNow}
        onInstall={onCliInstall}
        onChannelSet={onChannelSet}
        onAutoToggle={onCliAutoToggle}
        onEditEnvVars={onEditEnvVars ?? (() => {})}
      />
      <DesktopCard
        status={status}
        busy={busy}
        onCheck={onCheckNow}
        onInstall={onDesktopInstall}
        onAutoToggle={onDesktopAutoToggle}
      />
      <SettingsCard
        status={status}
        busy={busy}
        onCliNotifyToggle={onCliNotifyToggle}
        onCliNotifyOsToggle={onCliNotifyOsToggle}
        onDesktopNotifyToggle={onDesktopNotifyToggle}
        onDesktopNotifyOsToggle={onDesktopNotifyOsToggle}
        onMinimumVersionClear={onMinimumVersionClear}
      />
    </div>
  );
}
