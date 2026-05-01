import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../../api";
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

function formatRelativeTime(unix: number | null): string {
  if (!unix) return "never";
  const now = Date.now() / 1000;
  const dt = now - unix;
  if (dt < 60) return "just now";
  if (dt < 3600) return `${Math.round(dt / 60)} min ago`;
  if (dt < 86400) return `${Math.round(dt / 3600)} h ago`;
  return `${Math.round(dt / 86400)} d ago`;
}

function StatusBadge({ comparison }: { comparison: Comparison }) {
  switch (comparison) {
    case "older":
      return (
        <Tag tone="warn">
          <Glyph g={NF.download} />
          update available
        </Tag>
      );
    case "equal":
      return (
        <Tag tone="ok">
          <Glyph g={NF.check} />
          up to date
        </Tag>
      );
    case "newer":
      return <Tag tone="neutral">newer than channel</Tag>;
    default:
      return <Tag tone="neutral">unknown</Tag>;
  }
}

function CliCard({
  status,
  busy,
  onCheck,
  onInstall,
  onChannelSet,
  onAutoToggle,
}: {
  status: UpdatesStatusDto;
  busy: boolean;
  onCheck: () => void;
  onInstall: () => void;
  onChannelSet: (channel: "latest" | "stable") => void;
  onAutoToggle: (v: boolean) => void;
}) {
  const cli = status.cli;
  const active = cli.installs.find((i) => i.is_active);
  const installed = active?.version ?? null;
  const latest = cli.latest_remote ?? cli.last_known;
  const comparison = compareVersions(installed, latest);

  const disabledReason = cli.cc_settings.disable_updates
    ? "DISABLE_UPDATES=1 set in ~/.claude/settings.json"
    : !active
      ? "no active `claude` binary on PATH"
      : null;

  return (
    <Card>
      <CardHeader title="CC CLI" subtitle={`installed: ${installed ?? "unknown"}`} badge={<StatusBadge comparison={comparison} />} />
      <Row label="Latest">
        {latest ?? <em style={{ color: "var(--fg-faint)" }}>network probe failed</em>}
        {cli.last_check_unix && (
          <Sub>checked {formatRelativeTime(cli.last_check_unix)}</Sub>
        )}
      </Row>
      <Row label="Channel">
        <ChannelToggle
          value={cli.channel}
          onChange={onChannelSet}
          minimumVersion={cli.cc_settings.minimum_version}
        />
      </Row>
      <Row label="Auto-update">
        <ToggleWithLabel
          on={status.settings.cli.force_update_on_check}
          onChange={onAutoToggle}
          disabled={cli.cc_settings.disable_updates}
          label={
            cli.cc_settings.disable_updates
              ? "blocked by DISABLE_UPDATES=1"
              : "run `claude update` whenever a new version is detected"
          }
        />
      </Row>
      {cli.cc_settings.minimum_version && (
        <Row label="Floor">
          <code style={{ fontSize: "var(--fs-xs)" }}>
            minimumVersion = {cli.cc_settings.minimum_version}
          </code>
        </Row>
      )}
      {cli.cc_settings.disable_autoupdater && (
        <Warning>
          <Glyph g={NF.warn} />
          DISABLE_AUTOUPDATER=1 set — background auto-updates are off
        </Warning>
      )}
      {cli.cc_settings.disable_updates && (
        <Warning>
          <Glyph g={NF.warn} />
          DISABLE_UPDATES=1 set — manual updates are blocked too
        </Warning>
      )}
      {cli.running_count > 0 && (
        <Row label="Running">
          {cli.running_count} CC process{cli.running_count === 1 ? "" : "es"} active —
          symlink swap is safe (old processes keep their version)
        </Row>
      )}
      {cli.installs.length > 1 && (
        <Row label="Installs">
          <ul style={{ margin: 0, paddingLeft: "var(--sp-16)", listStyle: "none" }}>
            {cli.installs.map((i) => (
              <InstallRow key={i.binary_path} install={i} />
            ))}
          </ul>
        </Row>
      )}
      <Actions
        primary={{
          label: comparison === "older" ? "Update now" : "Update anyway",
          onClick: onInstall,
          disabled: busy || disabledReason !== null,
          glyph: NF.download,
          variant: comparison === "older" ? "solid" : "outline",
        }}
        secondary={{
          label: "Check now",
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
  const ds = status.desktop;
  const installed = ds.install?.version ?? null;
  const latest = ds.latest_remote;
  const comparison = compareVersions(installed, latest);

  const noInstall = !ds.install;
  const notManageable = ds.install && !ds.install.manageable;
  const disabledReason = noInstall
    ? "no Claude Desktop install detected"
    : notManageable
      ? `Desktop is managed by ${ds.install!.source} — Claudepot can't drive updates here`
      : ds.running
        ? "Desktop is currently running — quit it before updating"
        : null;

  return (
    <Card>
      <CardHeader
        title="Claude Desktop"
        subtitle={installed ? `installed: ${installed}` : "not installed"}
        badge={installed ? <StatusBadge comparison={comparison} /> : undefined}
      />
      {ds.install && (
        <Row label="Source">
          <SourceTag source={ds.install.source} />
        </Row>
      )}
      <Row label="Latest">
        {latest ?? <em style={{ color: "var(--fg-faint)" }}>network probe failed</em>}
        {ds.last_check_unix && (
          <Sub>checked {formatRelativeTime(ds.last_check_unix)}</Sub>
        )}
      </Row>
      <Row label="Status">
        <Tag tone={ds.running ? "warn" : "neutral"}>
          {ds.running ? "running" : "not running"}
        </Tag>
      </Row>
      <Row label="Auto-install">
        <ToggleWithLabel
          on={status.settings.desktop.auto_install_when_quit}
          onChange={onAutoToggle}
          label="when Desktop is not running"
        />
      </Row>
      <Actions
        primary={{
          label: comparison === "older" ? "Update now" : "Update anyway",
          onClick: onInstall,
          disabled: busy || disabledReason !== null,
          glyph: NF.download,
          variant: comparison === "older" ? "solid" : "outline",
        }}
        secondary={{
          label: "Check now",
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
  return (
    <Card>
      <CardHeader title="Settings" subtitle="Notifications and pinning" />
      <Row label="CLI notify">
        <span style={{ display: "flex", flexDirection: "column", gap: "var(--sp-6)" }}>
          <ToggleWithLabel
            on={status.settings.cli.notify_on_available}
            onChange={onCliNotifyToggle}
            label="tray badge when a new version is available"
          />
          <ToggleWithLabel
            on={status.settings.cli.notify_os_on_available}
            onChange={onCliNotifyOsToggle}
            label="OS notification (toast) when a new version is available"
          />
        </span>
      </Row>
      <Row label="Desktop notify">
        <span style={{ display: "flex", flexDirection: "column", gap: "var(--sp-6)" }}>
          <ToggleWithLabel
            on={status.settings.desktop.notify_on_available}
            onChange={onDesktopNotifyToggle}
            label="tray badge when a new version is available"
          />
          <ToggleWithLabel
            on={status.settings.desktop.notify_os_on_available}
            onChange={onDesktopNotifyOsToggle}
            label="OS notification (toast) when a new version is available"
          />
        </span>
      </Row>
      {status.cli.cc_settings.minimum_version && (
        <Row label="Pin">
          <span style={{ display: "inline-flex", alignItems: "center", gap: "var(--sp-8)" }}>
            <code style={{ fontSize: "var(--fs-xs)" }}>
              minimumVersion = {status.cli.cc_settings.minimum_version}
            </code>
            <Button
              size="sm"
              variant="outline"
              onClick={onMinimumVersionClear}
              disabled={busy}
            >
              clear
            </Button>
          </span>
        </Row>
      )}
      <Row label="Source">
        <span style={{ fontSize: "var(--fs-xs)", color: "var(--fg-faint)" }}>
          Channel + DISABLE_* keys live in <code>~/.claude/settings.json</code>{" "}
          (CC's own file). Notification toggles + cadence live in{" "}
          <code>~/.claudepot/updates.json</code>.
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
        cursor: disabled ? "not-allowed" : "pointer",
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
  return (
    <div style={{ display: "inline-flex", gap: "var(--sp-4)" }}>
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
            ? `minimumVersion=${minimumVersion} pin will be honored`
            : undefined
        }
      >
        stable
      </Button>
    </div>
  );
}

function SourceTag({ source }: { source: DesktopInstall["source"] }) {
  const labels: Record<DesktopInstall["source"], string> = {
    homebrew: "Homebrew Cask",
    "direct-dmg": "Direct download",
    setapp: "Setapp",
    "mac-app-store": "Mac App Store",
    "user-local": "User-local install",
  };
  return <Tag tone="neutral">{labels[source]}</Tag>;
}

function InstallRow({ install }: { install: CliInstall }) {
  const kindLabel: Record<CliInstall["kind"], string> = {
    "native-curl": "native (curl)",
    "npm-global": "npm",
    "homebrew-stable": "homebrew (stable)",
    "homebrew-latest": "homebrew (latest)",
    apt: "apt",
    dnf: "dnf",
    apk: "apk",
    "win-get": "winget",
    unknown: "unknown",
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
          active
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
  if (outcome.kind === "disabled" || outcome.kind === "up-to-date") {
    return null;
  }
  let tone: "ok" | "warn" | "error";
  let text: string;
  switch (outcome.kind) {
    case "installed":
      tone = "ok";
      text = `Auto-updated ${surface}${outcome.version ? ` to ${outcome.version}` : ""}.`;
      break;
    case "skipped":
      tone = "warn";
      text = `${surface} auto-update skipped: ${outcome.reason}`;
      break;
    case "failed":
      tone = "error";
      text = `${surface} auto-update failed: ${outcome.error}`;
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

export function UpdatesPanel() {
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
      setError(String(e));
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
  useEffect(() => {
    let active = true;
    const unlisten = listen("updates::cycle-complete", () => {
      if (active) {
        void refresh(false);
      }
    });
    return () => {
      active = false;
      void unlisten.then((fn) => fn());
    };
  }, [refresh]);

  const onChannelSet = useCallback(
    async (channel: "latest" | "stable") => {
      setBusy(true);
      setError(null);
      try {
        await api.updatesChannelSet(channel);
        setInfo(`Wrote autoUpdatesChannel=${channel} to ~/.claude/settings.json`);
        await refresh(false);
      } catch (e: unknown) {
        setError(String(e));
      } finally {
        setBusy(false);
      }
    },
    [refresh],
  );

  const onCliInstall = useCallback(async () => {
    setBusy(true);
    setError(null);
    setInfo(null);
    try {
      const res = await api.updatesCliInstall();
      setInfo(
        res.installed_after
          ? `Active install: ${res.installed_after}`
          : "Update completed",
      );
      await refresh(true);
    } catch (e: unknown) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, [refresh]);

  const onDesktopInstall = useCallback(async () => {
    setBusy(true);
    setError(null);
    setInfo(null);
    try {
      const res = await api.updatesDesktopInstall();
      setInfo(
        res.version_after
          ? `Installed Claude.app ${res.version_after} (via ${res.method})`
          : `Update completed via ${res.method}`,
      );
      await refresh(true);
    } catch (e: unknown) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, [refresh]);

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
        setError(String(e));
      } finally {
        setBusy(false);
      }
    },
    [refresh],
  );

  const onCliNotifyToggle = useCallback(
    (v: boolean) => setSettingsField({ cli_notify_on_available: v }, "Saved"),
    [setSettingsField],
  );
  const onCliNotifyOsToggle = useCallback(
    (v: boolean) => setSettingsField({ cli_notify_os_on_available: v }, "Saved"),
    [setSettingsField],
  );
  const onDesktopNotifyToggle = useCallback(
    (v: boolean) => setSettingsField({ desktop_notify_on_available: v }, "Saved"),
    [setSettingsField],
  );
  const onDesktopNotifyOsToggle = useCallback(
    (v: boolean) =>
      setSettingsField({ desktop_notify_os_on_available: v }, "Saved"),
    [setSettingsField],
  );
  const onCliAutoToggle = useCallback(
    (v: boolean) =>
      setSettingsField({ cli_force_update_on_check: v }, "Saved"),
    [setSettingsField],
  );
  const onDesktopAutoToggle = useCallback(
    (v: boolean) =>
      setSettingsField({ desktop_auto_install_when_quit: v }, "Saved"),
    [setSettingsField],
  );
  const onMinimumVersionClear = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      await api.updatesMinimumVersionSet(null);
      setInfo("Cleared minimumVersion in ~/.claude/settings.json");
      await refresh(false);
    } catch (e: unknown) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, [refresh]);

  const onCheckNow = useCallback(() => refresh(true), [refresh]);

  const banner = useMemo(() => {
    if (error) return { tone: "error" as const, text: error };
    if (info) return { tone: "ok" as const, text: info };
    return null;
  }, [error, info]);

  if (!status) {
    return (
      <div style={{ padding: "var(--sp-16)", color: "var(--fg-faint)" }}>
        Loading update status…
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
            Check now
          </Button>
        }
      >
        Updates
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
