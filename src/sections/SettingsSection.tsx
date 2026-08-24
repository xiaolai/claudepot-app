import {
  Fragment,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Trans, useTranslation } from "react-i18next";
import type { NfIcon } from "../icons";
import { api } from "../api";
import { applyLocalePreference, i18n } from "../lib/i18n";
import { formatDateTime } from "../lib/intl";
import type { CategoryMeta } from "../api/notification";
import type { CategoryPrefs as CategoryPrefsType } from "../api/settings";
import type { Category } from "../lib/notifications/types";
import {
  setCategoryPrefLocal,
  updateCategoryPref,
} from "../lib/notifications/prefs";
import {
  categoryGroupLabel,
  categoryLabel,
} from "../lib/notifications/labels";
import { BrandGithubMark } from "../components/primitives/BrandGithubMark";
import { Button } from "../components/primitives/Button";
import { ExternalLink } from "../components/primitives/ExternalLink";
import { Glyph } from "../components/primitives/Glyph";
import { SkeletonList } from "../components/primitives/Skeleton";
import { Tag } from "../components/primitives/Tag";
import { useSettingsActions } from "../hooks/useSettingsActions";
import { useTheme, type ThemeMode } from "../hooks/useTheme";
import { useAppState } from "../providers/AppStateProvider";
import { useUpdater, type CheckFrequency } from "../providers/UpdateProvider";
import { renderError, toastError } from "../lib/i18n-error";
import {
  dispatchOsNotification,
  getPermissionStatus,
  requestNotificationPermission,
  subscribePermissionStatus,
  type PermissionStatus,
} from "../lib/notify";
import { NF } from "../icons";
import {
  enabledSections,
  isSectionEnabled,
  OPTIONAL_SECTIONS_EVENT,
  setSectionEnabled,
} from "../lib/optionalSections";
import {
  SETTINGS_PANES,
  type SettingsPane,
  type SettingsPaneId,
} from "./settings/panes";
import { ScreenHeader } from "../shell/ScreenHeader";
import { HealthPane } from "./settings/HealthPane";
import { McpInstallerPane } from "./settings/McpInstallerPane";
import { NetworkPane } from "./settings/NetworkPane";
import { ProtectedPathsPane } from "./settings/ProtectedPathsPane";
import { RotationPane } from "./settings/RotationPane";
import { RemotePane } from "./settings/RemotePane";
import { RetentionPane } from "./settings/RetentionPane";
import { CleanupPane } from "./sessions/CleanupPane";
import { ArtifactLifecyclePane } from "./settings/ArtifactLifecyclePane";
import { CompanionArtifactToggle } from "./settings/CompanionArtifactToggle";
import { ExtendedThinkingToggle } from "./settings/ExtendedThinkingToggle";
import { AttributionControl } from "./settings/AttributionControl";
import { AvailableModelsPane } from "./settings/AvailableModelsPane";
import { FastModeToggle } from "./settings/FastModeToggle";
import { TrashDrawer } from "./sessions/TrashDrawer";
import type { AppStatus, CcIdentity } from "../types";
import { APP_VERSION } from "../version";
import {
  EVENT_SETTINGS_TAB,
  consumeSettingsTabHint,
  type SettingsTabEventDetail,
} from "../lib/networkPanelDeepLink";

// Pane table + the `Tab` union both live in `settings/panes.ts`. The
// ⌘K palette needs the same list to build "Open Settings → Retention"
// targets, and importing this module for it would pull the whole lazy
// Settings chunk into the main bundle.
//
// Cleanup re-landed here from the (now-removed) Sessions section's
// Cleanup sub-tab when the cross-project Sessions firehose was
// folded back into per-project browsing under `Projects`. Hosts the
// session prune flow + the trash drawer + the session-index rebuild
// utility — global maintenance operations on the on-disk transcript
// store. GC of stale projects still lives in Projects → Maintenance.
type Tab = SettingsPaneId;
const TAB_DEFS = SETTINGS_PANES;

// "Open on launch" targets. Derived from the section registry rather
// than hand-listed: the hand-listed version offered a "Sessions"
// option whose id no longer existed, so picking it silently landed
// the user on Accounts, and it was missing four real sections.
// Computed per render rather than at module load: a disabled
// section must not remain selectable after it is switched off.
const sectionOptions = () =>
  enabledSections().map((s) => ({
    value: s.id,
    label: i18n.t(s.labelKey, { ns: "shell" }),
  }));

export function SettingsSection() {
  const { t } = useTranslation("settings");
  const { pushToast } = useAppState();
  // Cold-mount path: read the sessionStorage hint set by the
  // NetworkUnreachablePanel before this section mounted.
  const [tab, setTab] = useState<Tab>(() => {
    const hint = consumeSettingsTabHint();
    if (hint && TAB_DEFS.some((d) => d.id === hint)) {
      return hint as Tab;
    }
    return "general";
  });
  const active = TAB_DEFS.find((d) => d.id === tab) ?? TAB_DEFS[0];

  // Hot-mount path: when this section is already mounted,
  // `setSection("settings")` is a no-op and the cold-mount
  // sessionStorage read won't re-fire. The CustomEvent reaches us
  // either way. See `src/lib/networkPanelDeepLink.ts`.
  useEffect(() => {
    const handler = (e: Event) => {
      const ce = e as CustomEvent<SettingsTabEventDetail>;
      const next = ce.detail?.tab;
      if (next && TAB_DEFS.some((d) => d.id === next)) {
        setTab(next as Tab);
      }
    };
    window.addEventListener(EVENT_SETTINGS_TAB, handler);
    return () => window.removeEventListener(EVENT_SETTINGS_TAB, handler);
  }, []);

  return (
    <>
      <ScreenHeader
        title={t("header.title")}
        subtitle={t("header.subtitle")}
      />

      <div style={{ flex: 1, display: "flex", minHeight: 0 }}>
        <SettingsNav active={tab} onSelect={setTab} />

        <main
          style={{
            flex: 1,
            minWidth: 0,
            overflow: "auto",
            padding: "var(--sp-24) var(--sp-32) var(--sp-40)",
          }}
        >
          <h2
            style={{
              fontSize: "var(--fs-lg)",
              fontWeight: 600,
              letterSpacing: "var(--ls-tight)",
              margin: 0,
              marginBottom: "var(--sp-16)",
            }}
          >
            {active.label}
          </h2>

          {tab === "general" && <GeneralPane pushToast={pushToast} />}
          {tab === "appearance" && <AppearancePane />}
          {tab === "notifications" && <NotificationsPane pushToast={pushToast} />}
          {tab === "network" && <NetworkPane pushToast={pushToast} />}
          {tab === "rotation" && <RotationPane pushToast={pushToast} />}
          {tab === "remote" && <RemotePane pushToast={pushToast} />}
          {tab === "retention" && <RetentionPane pushToast={pushToast} />}
          {tab === "health" && <HealthPane pushToast={pushToast} />}
          {tab === "mcp" && <McpInstallerPane pushToast={pushToast} />}
          {tab === "cleanup" && <CleanupTabPane pushToast={pushToast} />}
          {tab === "protected" && <ProtectedPathsPane pushToast={pushToast} />}
          {tab === "github" && <GithubPane pushToast={pushToast} />}
          {tab === "locks" && <LocksPane pushToast={pushToast} />}
          {tab === "diagnostics" && <DiagnosticsPane pushToast={pushToast} />}
          {tab === "about" && <AboutPane />}
        </main>
      </div>
    </>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                         Nav                                 */
/* ──────────────────────────────────────────────────────────── */

function SettingsNav({
  active,
  onSelect,
}: {
  active: Tab;
  onSelect: (t: Tab) => void;
}) {
  const { t } = useTranslation("settings");
  const groups: { label: string; items: readonly SettingsPane[] }[] = useMemo(
    () => [
      { label: "", items: TAB_DEFS.filter((d) => d.group === "core") },
      {
        label: t("header.advanced"),
        items: TAB_DEFS.filter((d) => d.group === "advanced"),
      },
    ],
    [t],
  );

  return (
    <aside
      style={{
        width: "var(--settings-nav-width)",
        flexShrink: 0,
        borderRight: "var(--bw-hair) solid var(--line)",
        background: "var(--bg-sunken)",
        padding: "var(--sp-16) 0",
        overflow: "auto",
      }}
    >
      {groups.map((group, gi) => (
        <div key={gi} style={{ marginBottom: "var(--sp-16)" }}>
          {group.label && (
            <div
              className="mono-cap"
              style={{
                padding: "var(--sp-6) var(--sp-16)",
                color: "var(--fg-ghost)",
              }}
            >
              {group.label}
            </div>
          )}
          {group.items.map((p) => {
            const isActive = p.id === active;
            return (
              <button
                key={p.id}
                type="button"
                onClick={() => onSelect(p.id)}
                aria-current={isActive ? "page" : undefined}
                className="pm-focus"
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "var(--sp-10)",
                  width: "100%",
                  padding: "var(--sp-7) var(--sp-16)",
                  fontSize: "var(--fs-sm)",
                  fontWeight: isActive ? 600 : 500,
                  color: isActive ? "var(--fg)" : "var(--fg-muted)",
                  background: isActive ? "var(--bg-active)" : "transparent",
                  border: "none",
                  borderLeft: isActive
                    ? "var(--bw-strong) solid var(--accent)"
                    : "var(--bw-strong) solid transparent",
                  textAlign: "left",
                }}
              >
                <Glyph
                  g={p.glyph}
                  color={isActive ? "var(--accent)" : "currentColor"}
                  style={{ fontSize: "var(--fs-base)" }}
                />
                <span>{p.label}</span>
              </button>
            );
          })}
        </div>
      ))}
    </aside>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                       General pane                          */
/* ──────────────────────────────────────────────────────────── */

function GeneralPane({
  pushToast,
}: {
  pushToast: (t: "info" | "error", msg: string) => void;
}) {
  const [startSection, setStartSection] = useState<string>(() => {
    try {
      return localStorage.getItem("claudepot.startSection") ?? "accounts";
    } catch {
      return "accounts";
    }
  });
  const { t } = useTranslation();
  const { t: ts } = useTranslation("settings");
  const [hideDock, setHideDock] = useState<boolean | null>(null);
  const [showWindowOnStartup, setShowWindowOnStartup] = useState<
    boolean | null
  >(null);
  const [launchAtLogin, setLaunchAtLogin] = useState<boolean | null>(null);
  const [isMac, setIsMac] = useState<boolean>(false);
  // null = follow system; undefined = not yet loaded.
  const [localePref, setLocalePref] = useState<string | null | undefined>(
    undefined,
  );
  // Mirrors `optionalSections`, and re-reads on its event so the
  // switch stays correct when ⌃⌥⌘B is pressed while this pane is open.
  const [boardsOn, setBoardsOn] = useState(() => isSectionEnabled("boards"));
  useEffect(() => {
    const sync = () => setBoardsOn(isSectionEnabled("boards"));
    window.addEventListener(OPTIONAL_SECTIONS_EVENT, sync);
    return () => window.removeEventListener(OPTIONAL_SECTIONS_EVENT, sync);
  }, []);

  // Reconcile a saved launch target that points at a section the user
  // has since switched off. Without this the <select> holds a value
  // absent from its own options — a controlled input showing nothing —
  // and the app would try to launch into a section that is gone.
  useEffect(() => {
    const ids = enabledSections().map((s) => s.id);
    if (ids.includes(startSection)) return;
    setStartSection("accounts");
    try {
      localStorage.setItem("claudepot.startSection", "accounts");
    } catch {
      /* storage unavailable — the in-memory value still reconciles */
    }
  }, [startSection, boardsOn]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [prefs, status, { isEnabled }] = await Promise.all([
          api.preferencesGet(),
          api.appStatus(),
          import("@tauri-apps/plugin-autostart"),
        ]);
        if (cancelled) return;
        setHideDock(prefs.hide_dock_icon);
        setShowWindowOnStartup(prefs.show_window_on_startup);
        setLocalePref(prefs.locale ?? null);
        setIsMac(status.platform === "macos");
        try {
          setLaunchAtLogin(await isEnabled());
        } catch {
          setLaunchAtLogin(false);
        }
      } catch (e) {
        if (!cancelled)
          pushToast("error", renderError(e, ts("shared.prefsLoadFailed")));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [pushToast, ts]);

  const changeStart = useCallback(
    (v: string) => {
      setStartSection(v);
      try {
        localStorage.setItem("claudepot.startSection", v);
        // The translated label, not the stored id: `v` is the
        // localStorage compat value ("events", "third-party"), so
        // interpolating it rendered `启动时打开：accounts`.
        pushToast(
          "info",
          ts("general.openOnLaunchToast", {
            value:
              sectionOptions().find((o) => o.value === v)?.label ?? v,
          }),
        );
      } catch {
        // best-effort persistence
      }
    },
    [pushToast, ts],
  );

  const changeLocale = useCallback(
    async (v: string) => {
      const next = v === "system" ? null : v;
      const prev = localePref;
      setLocalePref(next);
      try {
        // Persist first — preferences.json is authoritative — then
        // apply to the live instance + boot mirror.
        await api.preferencesSetLocale(next);
        await applyLocalePreference(next);
      } catch (e) {
        setLocalePref(prev ?? null);
        pushToast("error", renderError(e, t("language.changeFailed")));
      }
    },
    [localePref, pushToast, t],
  );

  const toggleHideDock = useCallback(
    async (next: boolean) => {
      const prev = hideDock;
      setHideDock(next);
      try {
        await api.preferencesSetHideDockIcon(next);
        pushToast(
          "info",
          next
            ? ts("general.hideDock.hiddenToast")
            : ts("general.hideDock.restoredToast"),
        );
      } catch (e) {
        setHideDock(prev);
        pushToast("error", renderError(e, ts("shared.toggleFailed")));
      }
    },
    [hideDock, pushToast, ts],
  );

  const toggleShowWindowOnStartup = useCallback(
    async (next: boolean) => {
      const prev = showWindowOnStartup;
      setShowWindowOnStartup(next);
      try {
        await api.preferencesSetShowWindowOnStartup(next);
      } catch (e) {
        setShowWindowOnStartup(prev);
        pushToast("error", renderError(e, ts("shared.toggleFailed")));
      }
    },
    [showWindowOnStartup, pushToast, ts],
  );

  const toggleLaunchAtLogin = useCallback(
    async (next: boolean) => {
      const prev = launchAtLogin;
      setLaunchAtLogin(next);
      try {
        const mod = await import("@tauri-apps/plugin-autostart");
        if (next) await mod.enable();
        else await mod.disable();
      } catch (e) {
        setLaunchAtLogin(prev);
        pushToast("error", renderError(e, ts("general.launchAtLogin.error")));
      }
    },
    [launchAtLogin, pushToast, ts],
  );

  return (
    <>
    <SettingsGroup
      desc={ts("general.groupStartupDesc")}
    >
      <Row label={ts("general.openOnLaunch")}>
        <select
          value={startSection}
          onChange={(e) => changeStart(e.target.value)}
          style={selectStyle}
        >
          {sectionOptions().map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
      </Row>
      <Row label={t("language.label")} hint={t("language.hint")}>
        <select
          value={localePref === undefined ? "system" : (localePref ?? "system")}
          onChange={(e) => changeLocale(e.target.value)}
          disabled={localePref === undefined}
          style={selectStyle}
        >
          <option value="system">{t("language.system")}</option>
          <option value="en">{t("language.en")}</option>
          <option value="zh-CN">{t("language.zh-CN")}</option>
        </select>
      </Row>
      <Row
        label={ts("general.showBoards.label")}
        hint={ts("general.showBoards.hint")}
      >
        <Toggle
          on={boardsOn}
          onChange={(next) => {
            // Mirror what the store ACTUALLY reached, not what was
            // asked for. `setSectionEnabled` already returns the real
            // state precisely so a failed write cannot leave this
            // switch disagreeing with the navigation.
            setBoardsOn(setSectionEnabled("boards", next));
          }}
        />
      </Row>
      <Row
        label={ts("general.launchAtLogin.label")}
        hint={ts("general.launchAtLogin.hint")}
      >
        <Toggle
          on={launchAtLogin === true}
          onChange={toggleLaunchAtLogin}
        />
      </Row>
      <Row
        label={ts("general.showWindow.label")}
        hint={ts("general.showWindow.hint")}
      >
        <Toggle
          on={showWindowOnStartup === true}
          onChange={toggleShowWindowOnStartup}
        />
      </Row>
      {isMac && (
        <Row
          label={ts("general.hideDock.label")}
          hint={ts("general.hideDock.hint")}
        >
          <Toggle on={hideDock === true} onChange={toggleHideDock} />
        </Row>
      )}
      {/* Developer mode: hidden from the UI on purpose. Toggle is
          ⌃⌥⌘L (Ctrl+Alt+Cmd+L). The four-modifier combo is
          unreachable by accident and matches macOS's own
          deep-system-toggle convention (e.g. ⌃⌥⌘8 inverts colors).
          Wired in `App.tsx` so it works from any section. A toast
          confirms the new state since the toggle has no visual
          surface to mirror it. */}
    </SettingsGroup>
    <SettingsGroup desc={ts("general.groupCcDesc")}>
      <CompanionArtifactToggle pushToast={pushToast} />
      <ExtendedThinkingToggle pushToast={pushToast} />
      <FastModeToggle pushToast={pushToast} />
      <AvailableModelsPane pushToast={pushToast} />
      <AttributionControl pushToast={pushToast} />
    </SettingsGroup>
    </>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                     Appearance pane                         */
/* ──────────────────────────────────────────────────────────── */

function AppearancePane() {
  const { t } = useTranslation("settings");
  const { mode, resolved, setMode } = useTheme();
  const options: { value: ThemeMode; label: string; glyph?: NfIcon }[] = [
    { value: null, label: t("appearance.system"), glyph: NF.cpu },
    { value: "light", label: t("appearance.light"), glyph: NF.sun },
    { value: "dark", label: t("appearance.dark"), glyph: NF.moon },
  ];
  return (
    <SettingsGroup desc={t("appearance.groupDesc")}>
      <Row label={t("appearance.theme")}>
        <div
          style={{
            display: "flex",
            gap: "var(--sp-2)",
            padding: "var(--sp-2)",
            background: "var(--bg-sunken)",
            border: "var(--bw-hair) solid var(--line)",
            borderRadius: "var(--r-2)",
          }}
        >
          {options.map((opt) => {
            const current = mode === opt.value;
            return (
              <button
                key={String(opt.value ?? "system")}
                type="button"
                onClick={() => setMode(opt.value)}
                aria-pressed={current}
                className="pm-focus"
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "var(--sp-6)",
                  padding: "var(--sp-4) var(--sp-10)",
                  fontSize: "var(--fs-xs)",
                  fontWeight: 500,
                  letterSpacing: "var(--ls-wide)",
                  textTransform: "uppercase",
                  color: current ? "var(--fg)" : "var(--fg-muted)",
                  background: current
                    ? "var(--bg-raised)"
                    : "transparent",
                  border: current
                    ? "var(--bw-hair) solid var(--line)"
                    : "var(--bw-hair) solid transparent",
                  borderRadius: "var(--r-1)",
                }}
              >
                {opt.glyph && <Glyph g={opt.glyph} />}
                {opt.label}
              </button>
            );
          })}
        </div>
      </Row>
      <Row label={t("appearance.active.label")} hint={t("appearance.active.hint")}>
        <Tag tone="accent" glyph={resolved === "dark" ? NF.moon : NF.sun}>
          {resolved === "dark"
            ? t("appearance.resolvedDark")
            : t("appearance.resolvedLight")}
        </Tag>
      </Row>
    </SettingsGroup>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                        Locks pane                           */
/* ──────────────────────────────────────────────────────────── */

function LocksPane({
  pushToast,
}: {
  pushToast: (t: "info" | "error", msg: string) => void;
}) {
  const { t } = useTranslation("settings");
  const gc = useSettingsActions(pushToast);
  return (
    <SettingsGroup desc={t("locks.desc")}>
      <Row label={t("locks.pathLabel")}>
        <input
          type="text"
          placeholder="/absolute/path/to/lockfile"
          value={gc.lockPath}
          onChange={(e) => gc.setLockPath(e.target.value)}
          style={{
            ...inputStyle,
            minWidth: "var(--filter-input-width)",
            width: "100%",
          }}
        />
      </Row>
      <div style={actionsStyle}>
        <Button
          variant="solid"
          danger
          onClick={gc.breakLock}
          disabled={gc.lockBusy || !gc.lockPath.trim()}
          title={t("locks.breakTitle")}
        >
          {t("locks.breakButton")}
        </Button>
        {(gc.lockBusy || !gc.lockPath.trim()) && (
          <DisabledReason>
            {gc.lockBusy ? t("locks.breaking") : t("locks.enterPath")}
          </DisabledReason>
        )}
      </div>
    </SettingsGroup>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                    Diagnostics pane                         */
/* ──────────────────────────────────────────────────────────── */

function DiagnosticsPane({
  pushToast,
}: {
  pushToast: (t: "info" | "error", msg: string) => void;
}) {
  const { t } = useTranslation("settings");
  const [appStatus, setAppStatus] = useState<AppStatus | null>(null);
  const [ccIdentity, setCcIdentity] = useState<CcIdentity | null>(null);
  const [diagBusy, setDiagBusy] = useState(false);

  const diagTokenRef = useRef(0);
  const diagMountedRef = useRef(true);
  useEffect(() => {
    diagMountedRef.current = true;
    return () => {
      diagMountedRef.current = false;
    };
  }, []);

  const loadDiagnostics = useCallback(async () => {
    const myToken = ++diagTokenRef.current;
    setDiagBusy(true);
    try {
      const [s, cc] = await Promise.all([
        api.appStatus(),
        api.currentCcIdentity(),
      ]);
      if (!diagMountedRef.current || myToken !== diagTokenRef.current)
        return;
      setAppStatus(s);
      setCcIdentity(cc);
    } catch (e) {
      if (!diagMountedRef.current || myToken !== diagTokenRef.current)
        return;
      pushToast("error", renderError(e, t("diagnostics.loadFailed")));
    } finally {
      if (diagMountedRef.current && myToken === diagTokenRef.current) {
        setDiagBusy(false);
      }
    }
  }, [pushToast, t]);

  useEffect(() => {
    loadDiagnostics();
  }, [loadDiagnostics]);

  const copyDiagnostics = useCallback(() => {
    if (!appStatus) return;
    const lines = [
      `Claudepot diagnostics`,
      `Platform:          ${appStatus.platform}/${appStatus.arch}`,
      `CLI active:        ${appStatus.cli_active_email ?? "—"}`,
      `Desktop active:    ${appStatus.desktop_active_email ?? "—"}`,
      `Desktop installed: ${appStatus.desktop_installed ? "yes" : "no"}`,
      `Accounts:          ${appStatus.account_count}`,
      `Data dir:          ${appStatus.data_dir}`,
      `CC identity:       ${ccIdentity?.email ?? "(not signed in)"}`,
      ...(ccIdentity?.error ? [`CC identity error: ${ccIdentity.error}`] : []),
      ...(ccIdentity?.verified_at
        ? [`CC verified at:    ${ccIdentity.verified_at}`]
        : []),
    ];
    void navigator.clipboard
      .writeText(lines.join("\n"))
      .then(() => pushToast("info", t("diagnostics.copied")))
      .catch((err) =>
        pushToast("error", renderError(err, t("shared.copyFailed"))),
      );
  }, [appStatus, ccIdentity, pushToast, t]);

  return (
    <SettingsGroup desc={t("diagnostics.groupDesc")}>
      {appStatus ? (
        <dl style={gridStyle}>
          <Kv label={t("diagnostics.platform")} value={`${appStatus.platform}/${appStatus.arch}`} mono />
          <Kv label={t("diagnostics.cliActive")} value={appStatus.cli_active_email ?? "—"} />
          <Kv label={t("diagnostics.desktopActive")} value={appStatus.desktop_active_email ?? "—"} />
          <Kv
            label={t("diagnostics.desktopInstalled")}
            value={
              appStatus.desktop_installed
                ? t("diagnostics.yes")
                : t("diagnostics.no")
            }
          />
          <Kv label={t("diagnostics.accounts")} value={String(appStatus.account_count)} />
          <Kv label={t("diagnostics.dataDir")} value={appStatus.data_dir} mono />
          <Kv
            label={t("diagnostics.ccIdentity")}
            value={ccIdentity?.email ?? t("diagnostics.notSignedIn")}
          />
          {ccIdentity?.error && (
            <Kv label={t("diagnostics.ccError")} value={ccIdentity.error} mono tone="warn" />
          )}
        </dl>
      ) : (
        <SkeletonList rows={4} />
      )}
      <div style={actionsStyle}>
        <Button
          variant="subtle"
          glyph={NF.refresh}
          onClick={loadDiagnostics}
          disabled={diagBusy}
        >
          {t("shared.refresh")}
        </Button>
        <Button
          variant="ghost"
          glyph={NF.copy}
          onClick={copyDiagnostics}
          disabled={!appStatus}
        >
          {t("diagnostics.copy")}
        </Button>
      </div>
    </SettingsGroup>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                        About pane                           */
/* ──────────────────────────────────────────────────────────── */

function AboutPane() {
  const { t } = useTranslation("settings");
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-24)" }}>
      <SettingsGroup>
        <dl style={gridStyle}>
          <Kv
            label={t("about.app")}
            value={
              <span>
                clau<span style={{ color: "var(--accent)" }}>depot</span>
              </span>
            }
          />
          <Kv label={t("about.version")} value={APP_VERSION} mono />
          <Kv
            label={t("about.website")}
            value={
              <span
                style={{
                  display: "inline-flex",
                  alignItems: "center",
                  gap: "var(--sp-6)",
                }}
              >
                <Glyph
                  g={NF.globe}
                  color="var(--fg-muted)"
                  style={{ fontSize: "var(--fs-base)" }}
                />
                <ExternalLink href="https://claudepot.com/app">
                  claudepot.com/app
                </ExternalLink>
              </span>
            }
          />
          <Kv
            label={t("about.author")}
            value={
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "var(--sp-12)",
                  flexWrap: "wrap",
                }}
              >
                <span
                  style={{
                    display: "inline-flex",
                    alignItems: "center",
                    gap: "var(--sp-6)",
                  }}
                >
                  <BrandGithubMark
                    aria-label={t("about.githubAria")}
                    style={{
                      width: "var(--fs-base)",
                      height: "var(--fs-base)",
                      color: "var(--fg-muted)",
                    }}
                  />
                  <ExternalLink href="https://github.com/xiaolai">
                    github.com/xiaolai
                  </ExternalLink>
                </span>
                <span
                  style={{
                    display: "inline-flex",
                    alignItems: "center",
                    gap: "var(--sp-6)",
                  }}
                >
                  <Glyph
                    g={NF.globe}
                    color="var(--fg-muted)"
                    style={{ fontSize: "var(--fs-base)" }}
                  />
                  <ExternalLink href="https://lixiaolai.com">
                    lixiaolai.com
                  </ExternalLink>
                </span>
              </div>
            }
          />
          <Kv label={t("about.publisher")} value="HANDO K.K." />
          <Kv label={t("about.design")} value="paper-mono" />
        </dl>
        <p
          style={{
            margin: "var(--sp-8) 0 0",
            fontSize: "var(--fs-xs)",
            color: "var(--fg-faint)",
            lineHeight: "var(--lh-body)",
          }}
        >
          <Trans
            ns="settings"
            i18nKey="about.signingNote"
            components={{ em: <em /> }}
          />
        </p>
      </SettingsGroup>
      <UpdatesPane />
    </div>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                     Updates sub-pane                        */
/* ──────────────────────────────────────────────────────────── */

/** Format a number of bytes as MB with one decimal. */
function formatMB(n: number): string {
  return (n / 1024 / 1024).toFixed(1);
}

/** Render the local datetime of a string the updater plugin handed us. */
function formatLastChecked(at: number | null): string {
  if (!at) return i18n.t("updates.never", { ns: "settings" });
  return formatDateTime(at);
}

function UpdatesPane() {
  const { t } = useTranslation("settings");
  const {
    supported,
    status,
    updateInfo,
    downloadProgress,
    error,
    isSkipped,
    stranded,
    autoCheckEnabled,
    setAutoCheckEnabled,
    checkFrequency,
    setCheckFrequency,
    lastCheckedAt,
    releaseChannel,
    setReleaseChannel,
    checkNow,
    downloadAndInstall,
    applyUpdate,
    skipThisVersion,
    resetSkip,
  } = useUpdater();

  // Platform probe in flight — render nothing rather than a flicker
  // of unavailable controls.
  if (supported === null) return null;

  // Linux .deb / system install: in-place updates would race apt, so
  // the in-app updater is gated off. Surface a single-row hint
  // pointing at the Releases page so the user knows where to go.
  if (supported === false) {
    return (
      <SettingsGroup desc={t("updates.unsupportedDesc")}>
        <Row label={t("updates.rowLabel")}>
          <ExternalLink href="https://github.com/xiaolai/claudepot-app/releases">
            github.com/xiaolai/claudepot-app/releases
          </ExternalLink>
        </Row>
      </SettingsGroup>
    );
  }

  const checkDisabled =
    status === "checking" ||
    status === "downloading" ||
    status === "ready";

  const showAvailableCard =
    !!updateInfo &&
    !isSkipped &&
    (status === "available" ||
      status === "downloading" ||
      status === "ready");

  return (
    <SettingsGroup desc={t("updates.groupDesc")}>
      {/* Status row + manual trigger. The "Check now" button only
          renders for user-actionable states. While the badge is
          showing a transient state ("Checking…", "Downloading…",
          "Ready to install — restart Claudepot"), there's nothing
          for the user to do AND the badge already conveys that
          state, so adding a second label next to it just duplicates
          the same word. The detailed download card below carries
          the progress percentage. */}
      <Row label={t("updates.rowLabel")}>
        <div style={{ display: "flex", alignItems: "center", gap: "var(--sp-12)" }}>
          <UpdateStatusBadge
            status={status}
            updateInfo={updateInfo}
            error={error}
            isSkipped={isSkipped}
            stranded={stranded}
          />
          {!checkDisabled && (
            <Button variant="ghost" onClick={() => void checkNow()}>
              {t("updates.checkNow")}
            </Button>
          )}
        </div>
      </Row>

      {/* Available / downloading / ready card. Single card switches
          its primary action by status; we never stack two cards. */}
      {showAvailableCard && updateInfo && (
        <UpdateAvailableCard
          info={updateInfo}
          status={status}
          progress={downloadProgress}
          onDownload={() => void downloadAndInstall()}
          onSkip={skipThisVersion}
          onApply={() => void applyUpdate()}
        />
      )}

      {/* When the user has skipped a version, surface a small
          inline note + an "undo" so they can change their mind. */}
      {updateInfo && isSkipped && (
        <Row
          label={t("updates.skipped.label")}
          hint={t("updates.skipped.hint", { version: updateInfo.version })}
        >
          <Button variant="ghost" onClick={resetSkip}>
            {t("updates.showAgain")}
          </Button>
        </Row>
      )}

      <Row
        label={t("updates.auto.label")}
        hint={t("updates.auto.hint")}
      >
        <Toggle on={autoCheckEnabled} onChange={setAutoCheckEnabled} />
      </Row>

      <Row
        label={t("updates.frequency.label")}
        hint={!autoCheckEnabled ? t("updates.frequency.hint") : undefined}
      >
        <select
          value={checkFrequency}
          onChange={(e) =>
            setCheckFrequency(e.target.value as CheckFrequency)
          }
          disabled={!autoCheckEnabled}
          style={selectStyle}
        >
          <option value="startup">{t("updates.frequency.startup")}</option>
          <option value="daily">{t("updates.frequency.daily")}</option>
          <option value="weekly">{t("updates.frequency.weekly")}</option>
          <option value="manual">{t("updates.frequency.manual")}</option>
        </select>
      </Row>

      <Row
        label={t("updates.channel.label")}
        hint={
          releaseChannel === "beta"
            ? t("updates.channel.betaHint")
            : t("updates.channel.stableHint")
        }
      >
        {releaseChannel === null ? (
          <span
            style={{ fontSize: "var(--fs-sm)", color: "var(--fg-faint)" }}
          >
            {t("shared.loading")}
          </span>
        ) : (
          <select
            value={releaseChannel}
            onChange={(e) =>
              setReleaseChannel(e.target.value as "stable" | "beta")
            }
            style={selectStyle}
            aria-label={t("updates.channel.aria")}
          >
            <option value="stable">{t("updates.channel.stable")}</option>
            <option value="beta">{t("updates.channel.beta")}</option>
          </select>
        )}
      </Row>

      <Row label={t("updates.lastChecked")}>
        <span
          style={{ fontSize: "var(--fs-sm)", color: "var(--fg-muted)" }}
        >
          {formatLastChecked(lastCheckedAt)}
        </span>
      </Row>

      <Row label={t("updates.allReleases.label")} hint={t("updates.allReleases.hint")}>
        <ExternalLink href="https://github.com/xiaolai/claudepot-app/releases">
          github.com/xiaolai/claudepot-app/releases
        </ExternalLink>
      </Row>
    </SettingsGroup>
  );
}

function UpdateStatusBadge({
  status,
  updateInfo,
  error,
  isSkipped,
  stranded,
}: {
  status: ReturnType<typeof useUpdater>["status"];
  updateInfo: ReturnType<typeof useUpdater>["updateInfo"];
  error: string | null;
  isSkipped: boolean;
  stranded: ReturnType<typeof useUpdater>["stranded"];
}) {
  const { t } = useTranslation("settings");
  let glyph: NfIcon = NF.info;
  let color = "var(--fg-muted)";
  let label = t("updates.status.idle");

  if (status === "checking") {
    glyph = NF.refresh;
    color = "var(--fg-muted)";
    label = t("updates.status.checking");
  } else if (status === "up-to-date" && stranded) {
    // Beta → Stable switch while running a prerelease newer than the
    // stable channel's current release: "latest version" would be
    // factually wrong — nothing was offered, but the user is not on
    // the latest stable either.
    glyph = NF.info;
    color = "var(--fg-muted)";
    label = stranded.stableVersion
      ? t("updates.status.strandedVersioned", {
          version: stranded.stableVersion,
        })
      : t("updates.status.stranded");
  } else if (status === "up-to-date") {
    glyph = NF.check;
    color = "var(--ok)";
    label = t("updates.status.latest");
  } else if (status === "available" && updateInfo) {
    glyph = NF.download;
    color = "var(--accent)";
    label = isSkipped
      ? t("updates.status.skippedVersion", { version: updateInfo.version })
      : t("updates.status.available", { version: updateInfo.version });
  } else if (status === "downloading") {
    glyph = NF.download;
    color = "var(--fg-muted)";
    label = t("updates.downloading");
  } else if (status === "ready") {
    glyph = NF.check;
    color = "var(--ok)";
    label = t("updates.status.ready");
  } else if (status === "error") {
    glyph = NF.warn;
    color = "var(--danger)";
    label = error
      ? t("updates.status.checkFailedDetail", { error })
      : t("updates.status.checkFailed");
  }

  return (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: "var(--sp-6)",
        fontSize: "var(--fs-sm)",
        color,
      }}
      role="status"
      aria-live="polite"
    >
      <Glyph g={glyph} style={{ fontSize: "var(--fs-base)" }} />
      <span>{label}</span>
    </span>
  );
}

function UpdateAvailableCard({
  info,
  status,
  progress,
  onDownload,
  onSkip,
  onApply,
}: {
  info: NonNullable<ReturnType<typeof useUpdater>["updateInfo"]>;
  status: ReturnType<typeof useUpdater>["status"];
  progress: ReturnType<typeof useUpdater>["downloadProgress"];
  onDownload: () => void;
  onSkip: () => void;
  onApply: () => void;
}) {
  const { t } = useTranslation("settings");
  const total = progress?.total ?? 0;
  const downloaded = progress?.downloaded ?? 0;
  const pct = total > 0 ? Math.round((downloaded / total) * 100) : 0;

  return (
    <div
      style={{
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        padding: "var(--sp-14) var(--sp-16)",
        background: "var(--bg-raised)",
      }}
    >
      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "flex-start",
          gap: "var(--sp-16)",
        }}
      >
        <div style={{ flex: 1, minWidth: 0 }}>
          <div
            style={{
              display: "flex",
              alignItems: "baseline",
              gap: "var(--sp-8)",
            }}
          >
            <span
              style={{
                fontSize: "var(--fs-base)",
                fontWeight: 600,
                color: "var(--fg)",
              }}
            >
              v{info.version}
            </span>
            <span
              style={{
                fontSize: "var(--fs-xs)",
                color: "var(--fg-faint)",
              }}
            >
              {t("updates.card.current", { version: info.currentVersion })}
            </span>
            {info.pubDate && (
              <span
                style={{
                  fontSize: "var(--fs-xs)",
                  color: "var(--fg-faint)",
                }}
              >
                · {info.pubDate.slice(0, 10)}
              </span>
            )}
          </div>
          {info.notes && (
            <div
              style={{
                marginTop: "var(--sp-8)",
                fontSize: "var(--fs-sm)",
                color: "var(--fg-muted)",
                whiteSpace: "pre-wrap",
                maxHeight: "var(--sp-96)",
                overflow: "auto",
              }}
            >
              {info.notes}
            </div>
          )}
        </div>

        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: "var(--sp-6)",
            flexShrink: 0,
          }}
        >
          {status === "available" && (
            <>
              <Button variant="solid" onClick={onDownload}>
                {t("updates.card.download")}
              </Button>
              <Button variant="ghost" onClick={onSkip}>
                {t("updates.card.skip")}
              </Button>
            </>
          )}
          {status === "downloading" && (
            <Button variant="solid" disabled>
              {total > 0
                ? t("updates.card.downloadingPct", { pct })
                : t("updates.downloading")}
            </Button>
          )}
          {status === "ready" && (
            <Button variant="solid" onClick={onApply}>
              {t("updates.card.restart")}
            </Button>
          )}
        </div>
      </div>

      {status === "downloading" && (
        <div style={{ marginTop: "var(--sp-10)" }}>
          <div
            style={{
              display: "flex",
              justifyContent: "space-between",
              fontSize: "var(--fs-xs)",
              color: "var(--fg-faint)",
              marginBottom: "var(--sp-4)",
            }}
          >
            <span>{t("updates.downloading")}</span>
            <span>
              {formatMB(downloaded)} / {total > 0 ? formatMB(total) : "?"} MB
              {total > 0 && ` (${pct}%)`}
            </span>
          </div>
          <div
            style={{
              height: "var(--sp-4)",
              background: "var(--bg-active)",
              borderRadius: "var(--r-pill)",
              overflow: "hidden",
            }}
          >
            <div
              style={{
                width: `${pct}%`,
                height: "100%",
                background: "var(--accent)",
                transition: "width var(--dur-base) var(--ease-out)",
              }}
            />
          </div>
        </div>
      )}
    </div>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                         Cleanup pane                         */
/* ──────────────────────────────────────────────────────────── */

function CleanupTabPane({
  pushToast,
}: {
  pushToast: (kind: "info" | "error", text: string) => void;
}) {
  // Reuse the existing CleanupPane + TrashDrawer that previously
  // lived under Sessions → Cleanup. The two-pane layout (filter +
  // plan on the left, trash on the right) carries over verbatim;
  // the sub-tab outer chrome is gone since this is just a Settings
  // sub-tab now.
  //
  // `setToast` adapts CleanupPane's signature to SettingsSection's
  // (kind, text) API. The kind is passed by the emitter, which knows
  // it: this used to sniff the message for "failed:" / "couldn't …",
  // which worked only while those messages were English. After string
  // extraction a localized failure would have been classified as info
  // and shown in the wrong channel — the same defect as the
  // `auth rejected:` banner, one layer down.
  const setToast = useCallback(
    (msg: string, kind: "info" | "error" = "info") => pushToast(kind, msg),
    [pushToast],
  );
  // Bumped when CleanupPane dispatches a prune so the TrashDrawer
  // re-fetches and shows the newly-trashed entries. We deliberately
  // do NOT pass this tick as the drawer's `key` — that would force
  // a remount + drop the drawer's local state on every action — and
  // we don't bump on the drawer's own onChange (the drawer already
  // refreshes itself after restore/empty).
  const [trashTick, setTrashTick] = useState(0);
  // Single-column stack. The previous 2:1 row (CleanupPane | TrashDrawer)
  // had no reflow, so a narrow settings content column (~480px on a
  // 1000px window after sidebar + nav) squeezed the trash drawer's
  // header buttons and entry rows into an unusable strip. Stacking
  // gives every section the full content width and matches the
  // ArtifactLifecyclePane treatment below — hair-line top borders as
  // section separators, no padding-left guttering.
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-32)",
      }}
    >
      <CleanupPane
        onTrashChanged={() => setTrashTick((n) => n + 1)}
        setToast={setToast}
      />

      {/* `key={trashTick}` forces a remount whenever CleanupPane
          dispatches a prune so the drawer picks up the newly-
          trashed entries. We deliberately do NOT pass `onChange`
          here — the drawer already calls its own refresh() after
          restore / empty actions, so wiring it back to setTrashTick
          would double-bump and remount the drawer mid-action. */}
      <div
        style={{
          borderTop: "var(--bw-hair) solid var(--line)",
          paddingTop: "var(--sp-16)",
        }}
      >
        <TrashDrawer key={trashTick} />
      </div>

      {/* Artifact lifecycle (Disable + Trash for skills/agents/
          commands). */}
      <div
        style={{
          borderTop: "var(--bw-hair) solid var(--line)",
          paddingTop: "var(--sp-16)",
        }}
      >
        <ArtifactLifecyclePane
          pushToast={pushToast}
          // CleanupTabPane doesn't know about the active project
          // anchor — Settings is global. Project-scoped disabled
          // artifacts still surface when the user opened the same
          // project in Config; for the global Settings view we pass
          // null so only user-scope entries appear.
          projectRoot={null}
        />
      </div>

      {/* Diagnostic logs — reveal the rolling log directory. The
          GUI writes every `tracing` event and any panic there, so
          this is the entry point when the user is filing a "the
          app just quit" report. */}
      <div
        style={{
          borderTop: "var(--bw-hair) solid var(--line)",
          paddingTop: "var(--sp-16)",
        }}
      >
        <DiagnosticLogsPane pushToast={pushToast} />
      </div>
    </div>
  );
}

function DiagnosticLogsPane({
  pushToast,
}: {
  pushToast: (kind: "info" | "error", text: string) => void;
}) {
  const { t } = useTranslation("settings");
  const onReveal = useCallback(async () => {
    try {
      await api.revealLogsDir();
    } catch (e) {
      toastError(pushToast, t("logs.openFailed"), e);
    }
  }, [pushToast, t]);
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-8)",
      }}
    >
      <div
        style={{
          fontSize: "var(--fs-md)",
          fontWeight: 600,
          color: "var(--fg)",
        }}
      >
        {t("logs.title")}
      </div>
      <div
        style={{
          fontSize: "var(--fs-sm)",
          color: "var(--fg-muted)",
          lineHeight: 1.5,
        }}
      >
        <Trans
          ns="settings"
          i18nKey="logs.desc"
          components={{ code: <code /> }}
        />
      </div>
      <div>
        <Button
          variant="ghost"
          glyph={NF.folder}
          onClick={onReveal}
        >
          {t("logs.reveal")}
        </Button>
      </div>
    </div>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                      Notifications pane                      */
/* ──────────────────────────────────────────────────────────── */

function NotificationsPane({
  pushToast,
}: {
  pushToast: (k: "info" | "error", msg: string) => void;
}) {
  const { t } = useTranslation("settings");
  const [loaded, setLoaded] = useState(false);
  const [enabled, setEnabled] = useState(false);
  const [hideThinking, setHideThinking] = useState(true);
  const [notifyError, setNotifyError] = useState(false);
  const [notifyIdleDone, setNotifyIdleDone] = useState(false);
  const [notifyStuckMin, setNotifyStuckMin] = useState<number | null>(null);
  const [notifyOpDone, setNotifyOpDone] = useState(false);
  // Default-true to match the Preferences default; flipped to the
  // backend value once preferencesGet resolves. Avoids a render-flash
  // where the toggle briefly shows "off" for a feature that defaults on.
  const [notifyWaiting, setNotifyWaiting] = useState(true);
  // Mirror of preferences.notify_on_usage_thresholds. Default mirrors
  // the Rust default ([90]) so the chip group renders sensibly before
  // the first preferencesGet round-trips. Pre-2026-05 the default was
  // [80, 90]; trimmed to one threshold to cut toast volume in half
  // without removing the user's ability to add 80 back via this row.
  const [usageThresholds, setUsageThresholds] = useState<number[]>([90]);
  // Mirror of preferences.notify_on_sub_windows. Default false to
  // match Rust — the per-model 7-day sub-windows usually track the
  // umbrella for users near cap, so leaving them on triples the
  // 7-day toast volume for what most users experience as one cap.
  const [notifySubWindows, setNotifySubWindows] = useState(false);

  useEffect(() => {
    let cancelled = false;
    api
      .preferencesGet()
      .then((p) => {
        if (cancelled) return;
        setEnabled(p.activity_enabled);
        setHideThinking(p.activity_hide_thinking);
        setNotifyError(p.notify_on_error);
        setNotifyIdleDone(p.notify_on_idle_done);
        setNotifyStuckMin(p.notify_on_stuck_minutes);
        setNotifyOpDone(p.notify_on_op_done);
        setNotifyWaiting(p.notify_on_waiting);
        setUsageThresholds(p.notify_on_usage_thresholds ?? []);
        setNotifySubWindows(p.notify_on_sub_windows);
        setLoaded(true);
      })
      .catch((e) => {
        if (cancelled) return;
        pushToast("error", renderError(e, t("shared.prefsLoadFailed")));
        // Flip loaded anyway — otherwise the pane is stuck on
        // "Loading…" forever after one backend hiccup. Toggles stay
        // at their safe defaults (all off) until the user interacts.
        setLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, [pushToast, t]);

  const toggleEnabled = useCallback(
    async (next: boolean) => {
      const prev = enabled;
      setEnabled(next);
      try {
        await api.preferencesSetActivity({ enabled: next });
        if (next) await api.sessionLiveStart();
        else await api.sessionLiveStop();
        // Sessions section's transcript viewer + any other consumer
        // of `useActivityPrefs` picks up the change without polling.
        window.dispatchEvent(new CustomEvent("cp-activity-prefs-changed"));
        pushToast(
          "info",
          next ? t("notifications.activityOn") : t("notifications.activityOff"),
        );
      } catch (e) {
        setEnabled(prev);
        pushToast("error", renderError(e, t("shared.toggleFailed")));
      }
    },
    [enabled, pushToast, t],
  );

  const toggleHideThinking = useCallback(
    async (next: boolean) => {
      const prev = hideThinking;
      setHideThinking(next);
      try {
        await api.preferencesSetActivity({ hideThinking: next });
        window.dispatchEvent(new CustomEvent("cp-activity-prefs-changed"));
      } catch (e) {
        setHideThinking(prev);
        pushToast("error", renderError(e, t("shared.toggleFailed")));
      }
    },
    [hideThinking, pushToast, t],
  );

  const setNotifyBool = useCallback(
    async (
      key: "onError" | "onIdleDone" | "onOpDone" | "onWaiting",
      setter: (v: boolean) => void,
      prev: boolean,
      next: boolean,
    ) => {
      setter(next);
      try {
        await api.preferencesSetNotifications({ [key]: next });
      } catch (e) {
        setter(prev);
        pushToast("error", renderError(e, t("shared.toggleFailed")));
      }
    },
    [pushToast, t],
  );

  const toggleUsageThreshold = useCallback(
    // `pct`, not `t` — a `t` parameter would shadow the translation fn.
    async (pct: number) => {
      const prev = usageThresholds;
      const has = prev.includes(pct);
      const next = has
        ? prev.filter((x) => x !== pct)
        : [...prev, pct].sort((a, b) => a - b);
      setUsageThresholds(next);
      try {
        await api.preferencesSetNotifications({ onUsageThresholds: next });
      } catch (e) {
        setUsageThresholds(prev);
        pushToast("error", renderError(e, t("shared.saveFailed")));
      }
    },
    [usageThresholds, pushToast, t],
  );

  const toggleSubWindows = useCallback(
    async (next: boolean) => {
      const prev = notifySubWindows;
      setNotifySubWindows(next);
      try {
        await api.preferencesSetNotifications({ onSubWindows: next });
      } catch (e) {
        setNotifySubWindows(prev);
        pushToast("error", renderError(e, t("shared.toggleFailed")));
      }
    },
    [notifySubWindows, pushToast, t],
  );

  const setStuckMin = useCallback(
    async (raw: string) => {
      const parsed = raw === "" ? null : Number(raw);
      const normalized =
        parsed !== null && Number.isFinite(parsed) && parsed > 0
          ? Math.floor(parsed)
          : null;
      const prev = notifyStuckMin;
      setNotifyStuckMin(normalized);
      try {
        await api.preferencesSetNotifications({
          onStuckMinutes: normalized,
        });
      } catch (e) {
        setNotifyStuckMin(prev);
        pushToast("error", renderError(e, t("shared.saveFailed")));
      }
    },
    [notifyStuckMin, pushToast, t],
  );

  if (!loaded) {
    return <SkeletonList rows={5} />;
  }

  return (
    <>
      <SettingsGroup desc={t("notifications.groupActivityDesc")}>
        <Row
          label={t("notifications.enable.label")}
          hint={t("notifications.enable.hint")}
        >
          <Toggle on={enabled} onChange={toggleEnabled} />
        </Row>
        <Row
          label={t("notifications.hideThinking.label")}
          hint={t("notifications.hideThinking.hint")}
        >
          <Toggle
            on={hideThinking}
            onChange={toggleHideThinking}
          />
        </Row>
      </SettingsGroup>

      <SettingsGroup desc={t("notifications.groupAlertsDesc")}>
        <NotificationPermissionRow pushToast={pushToast} />
        <Row
          label={t("notifications.errorBurst.label")}
          hint={t("notifications.errorBurst.hint")}
        >
          <Toggle
            on={notifyError}
            onChange={(next) =>
              setNotifyBool("onError", setNotifyError, notifyError, next)
            }
          />
        </Row>
        <Row
          label={t("notifications.waiting.label")}
          hint={t("notifications.waiting.hint")}
        >
          <Toggle
            on={notifyWaiting}
            onChange={(next) =>
              setNotifyBool(
                "onWaiting",
                setNotifyWaiting,
                notifyWaiting,
                next,
              )
            }
          />
        </Row>
        <Row
          label={t("notifications.taskDone.label")}
          hint={t("notifications.taskDone.hint")}
        >
          <Toggle
            on={notifyIdleDone}
            onChange={(next) =>
              setNotifyBool(
                "onIdleDone",
                setNotifyIdleDone,
                notifyIdleDone,
                next,
              )
            }
          />
        </Row>
        <Row
          label={t("notifications.stuck.label")}
          hint={t("notifications.stuck.hint")}
        >
          <input
            type="number"
            min="1"
            step="1"
            inputMode="numeric"
            placeholder={t("notifications.stuck.off")}
            value={notifyStuckMin ?? ""}
            onChange={(e) => setStuckMin(e.target.value)}
            style={{
              ...selectStyle,
              width: "var(--input-width-compact)",
              textAlign: "right",
              fontVariantNumeric: "tabular-nums",
            }}
          />
        </Row>
        <Row
          label={t("notifications.usage.label")}
          hint={t("notifications.usage.hint")}
        >
          <UsageThresholdChips
            thresholds={usageThresholds}
            onToggle={toggleUsageThreshold}
          />
        </Row>
        <Row
          label={t("notifications.subWindows.label")}
          hint={t("notifications.subWindows.hint")}
        >
          <Toggle on={notifySubWindows} onChange={toggleSubWindows} />
        </Row>
        <Row
          label={t("notifications.opDone.label")}
          hint={t("notifications.opDone.hint")}
        >
          <Toggle
            on={notifyOpDone}
            onChange={(next) =>
              setNotifyBool(
                "onOpDone",
                setNotifyOpDone,
                notifyOpDone,
                next,
              )
            }
          />
        </Row>
        <Row
          label={t("notifications.test.label")}
          hint={t("notifications.test.hint")}
        >
          <Button
            variant="ghost"
            onClick={async () => {
              try {
                // Intentional direct call: this button tests the OS
                // dispatcher specifically. It bypasses emit() because
                // its purpose is to verify permissions and the
                // OS-banner pipeline, not to route through the
                // category-prefs gate (which would suppress the test
                // banner if the user had configEdited disabled).
                const ok = await dispatchOsNotification(
                  t("notifications.test.title"),
                  t("notifications.test.body"),
                  { ignoreFocus: true },
                );
                if (ok) {
                  pushToast("info", t("notifications.test.sent"));
                  return;
                }
                // `dispatchOsNotification` returns false for several
                // reasons besides denial — probe failure (Tauri
                // plugin not ready), unknown state, or a swallowed
                // sendNotification throw. Read the live status to
                // give the user the right remediation copy.
                const status = getPermissionStatus();
                if (status === "denied") {
                  pushToast("error", t("notifications.test.denied"));
                } else if (status === "not-requested") {
                  pushToast("info", t("notifications.test.notGranted"));
                } else {
                  pushToast("error", t("notifications.test.unreachable"));
                }
              } catch (e) {
                pushToast(
                  "error",
                  renderError(e, t("notifications.test.sendFailed")),
                );
              }
            }}
          >
            {t("notifications.test.button")}
          </Button>
        </Row>
      </SettingsGroup>

      {/*
        Phase 4 — per-category notification toggles. Surfaces the
        full Category enum from the Rust side via the
        `notification_categories_metadata` IPC. Categories that
        also have a legacy scalar (notify_on_*) are covered by the
        toggles above; this section surfaces the rest so the user
        can mute / unmute rotation, memory changes, banner
        transitions, and other categories that previously had no
        UI. The dual-write contract from Phase 1.5 keeps both
        forms in sync, so toggling above and toggling here both
        persist correctly.
      */}
      <CategoryPrefsListGroup pushToast={pushToast} />
    </>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*           Per-category notification toggles (Phase 4)         */
/* ──────────────────────────────────────────────────────────── */

/**
 * Reads the live `Category` metadata from the Rust side and
 * renders one row per category, grouped by priority tier. Each
 * row has an `enabled` toggle; clicking persists via
 * `preferencesCategoryPrefSet` and updates the local prefs cache
 * (which `emit()` reads on the next dispatch).
 *
 * Categories with legacy scalar mirrors (notify_on_*) are hidden
 * by default — they're already covered by the toggles above. A
 * disclosure expands the full list for users who want to see
 * everything.
 */
function CategoryPrefsListGroup({
  pushToast,
}: {
  pushToast: (k: "info" | "error", msg: string) => void;
}) {
  const { t } = useTranslation("settings");
  const [meta, setMeta] = useState<CategoryMeta[] | null>(null);
  const [prefs, setPrefs] = useState<Record<string, CategoryPrefsType> | null>(
    null,
  );
  const [showLegacy, setShowLegacy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void Promise.all([
      api.notificationCategoriesMetadata(),
      api.preferencesCategoryPrefsGet(),
    ])
      .then(([m, p]) => {
        if (cancelled) return;
        setMeta(m);
        setPrefs(p as Record<string, CategoryPrefsType>);
      })
      .catch((e) => {
        if (cancelled) return;
        pushToast(
          "error",
          renderError(e, t("notifications.categories.loadFailed")),
        );
      });
    return () => {
      cancelled = true;
    };
  }, [pushToast, t]);

  // Categories already represented by a scalar toggle above —
  // these stay hidden unless the user expands "show all".
  const legacyCategories = useMemo(
    () =>
      new Set<Category>([
        "sessionErrorBurst",
        "opDoneUnfocused",
        "sessionStuck",
        "sessionWaiting",
        "usageThreshold",
        "serviceStatusChanged",
      ]),
    [],
  );

  const rows = useMemo(() => {
    if (!meta) return [];
    return meta.filter((c) => showLegacy || !legacyCategories.has(c.id));
  }, [meta, showLegacy, legacyCategories]);

  const grouped = useMemo(() => {
    const g: Record<string, typeof rows> = {};
    for (const r of rows) {
      g[r.group] = g[r.group] ?? [];
      g[r.group].push(r);
    }
    return g;
  }, [rows]);

  const setEnabled = useCallback(
    async (id: Category, next: boolean) => {
      const cur = prefs?.[id] ?? { enabled: true, osOverride: null };
      const optimistic = { ...cur, enabled: next };
      setPrefs((p) => ({ ...(p ?? {}), [id]: optimistic }));
      setCategoryPrefLocal(id, optimistic);
      try {
        const confirmed = await updateCategoryPref(id, optimistic);
        setPrefs((p) => ({ ...(p ?? {}), [id]: confirmed }));
      } catch (e) {
        // Revert on failure.
        setPrefs((p) => ({ ...(p ?? {}), [id]: cur }));
        setCategoryPrefLocal(id, cur);
        pushToast("error", renderError(e, t("shared.toggleFailed")));
      }
    },
    [prefs, pushToast, t],
  );

  if (!meta || !prefs) {
    return (
      <SettingsGroup desc={t("notifications.categories.loadingDesc")}>
        <div />
      </SettingsGroup>
    );
  }

  return (
    <SettingsGroup desc={t("notifications.categories.desc")}>
      {Object.entries(grouped).map(([group, items]) => (
        <Fragment key={group}>
          <div
            style={{
              gridColumn: "1 / -1",
              fontSize: "var(--fs-xs)",
              color: "var(--fg-muted)",
              textTransform: "uppercase",
              letterSpacing: "var(--ls-wide)",
              marginTop: "var(--sp-12)",
            }}
          >
            {categoryGroupLabel(group)}
          </div>
          {items.map((c) => {
            const p = prefs[c.id] ?? { enabled: true, osOverride: null };
            return (
              <Row
                key={c.id}
                // Localized off the stable id; the IPC-shipped English
                // label is only the fallback (lib/notifications/labels).
                label={categoryLabel(c)}
                hint={`${c.priority.replace(/([A-Z])/g, " $1").trim()} — ${
                  legacyCategories.has(c.id)
                    ? t("notifications.categories.alsoAbove")
                    : ""
                }`}
              >
                <Toggle
                  on={p.enabled}
                  onChange={(next) => void setEnabled(c.id, next)}
                />
              </Row>
            );
          })}
        </Fragment>
      ))}
      <Row
        label={t("notifications.categories.showAll")}
        hint={t("notifications.categories.showAllHint")}
      >
        <Toggle on={showLegacy} onChange={setShowLegacy} />
      </Row>
    </SettingsGroup>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                  Usage threshold chip group                  */
/* ──────────────────────────────────────────────────────────── */

/**
 * Multi-select chip group for the `notify_on_usage_thresholds`
 * preference. Empty selection = feature off; the watcher early-exits
 * on an empty list and no events fire. Choices are deliberately
 * coarse (50 / 70 / 80 / 90 / 95) — usage utilization is a slow-moving
 * signal, finer granularity wouldn't change behaviour. Custom
 * thresholds are out of scope for v1; if a user needs one, the
 * preference field accepts arbitrary integers and a future "add
 * custom" affordance can be plugged in here.
 */
const USAGE_THRESHOLD_CHOICES = [50, 70, 80, 90, 95] as const;

function UsageThresholdChips({
  thresholds,
  onToggle,
}: {
  thresholds: number[];
  onToggle: (t: number) => void;
}) {
  return (
    <div style={{ display: "flex", gap: "var(--sp-6)", flexWrap: "wrap" }}>
      {USAGE_THRESHOLD_CHOICES.map((t) => {
        const on = thresholds.includes(t);
        return (
          <button
            key={t}
            type="button"
            onClick={() => onToggle(t)}
            aria-pressed={on}
            className="pm-focus"
            style={{
              padding: "var(--sp-2) var(--sp-8)",
              fontFamily: "inherit",
              fontSize: "var(--fs-xs)",
              fontVariantNumeric: "tabular-nums",
              borderRadius: "var(--r-1)",
              border: "var(--bw-hair) solid var(--line)",
              background: on ? "var(--accent-soft)" : "transparent",
              color: on ? "var(--accent-ink)" : "var(--fg)",
            }}
          >
            {t}%
          </button>
        );
      })}
    </div>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*               Notification permission status row             */
/* ──────────────────────────────────────────────────────────── */

/**
 * Surfaces the current OS notification permission state so a user
 * who toggles a notify_* preference with denied permission isn't
 * silently dropped. Subscribes to the singleton in `lib/notify.ts`
 * so the row updates in real time after a Request click.
 */
function NotificationPermissionRow({
  pushToast,
}: {
  pushToast: (k: "info" | "error", msg: string) => void;
}) {
  const { t } = useTranslation("settings");
  const [status, setStatus] = useState<PermissionStatus>(() =>
    getPermissionStatus(),
  );

  useEffect(() => subscribePermissionStatus(setStatus), []);

  const label = t("notifications.permission.label");
  let hint: string;
  switch (status) {
    case "granted":
      hint = t("notifications.permission.grantedHint");
      break;
    case "denied":
      hint = t("notifications.permission.deniedHint");
      break;
    case "not-requested":
      hint = t("notifications.permission.notRequestedHint");
      break;
    case "unknown":
    default:
      hint = t("notifications.permission.probingHint");
      break;
  }

  return (
    <Row label={label} hint={hint}>
      {status === "granted" && (
        <Tag>{t("notifications.permission.grantedTag")}</Tag>
      )}
      {status === "denied" && (
        <Tag tone="danger">{t("notifications.permission.deniedTag")}</Tag>
      )}
      {status === "not-requested" && (
        <Button
          variant="ghost"
          onClick={async () => {
            const next = await requestNotificationPermission();
            if (next === "granted") {
              pushToast("info", t("notifications.permission.enabled"));
            } else if (next === "denied") {
              pushToast("error", t("notifications.permission.deniedToast"));
            }
          }}
        >
          {t("notifications.permission.request")}
        </Button>
      )}
      {status === "unknown" && (
        <Tag tone="ghost">{t("notifications.permission.unknownTag")}</Tag>
      )}
    </Row>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                         GitHub                              */
/* ──────────────────────────────────────────────────────────── */

function GithubPane({
  pushToast,
}: {
  pushToast: (t: "info" | "error", msg: string) => void;
}) {
  const { t } = useTranslation("settings");
  const [status, setStatus] = useState<{
    present: boolean;
    last4: string | null;
    env_override: boolean;
  } | null>(null);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setStatus(await api.settingsGithubTokenGet());
    } catch (e) {
      // Route through toastError so the redactSecrets pipeline scrubs
      // any `sk-ant-*` / `ghp_*` blob the backend might echo back. The
      // toast lingers in the DOM (and now in the status-bar echo) so
      // raw stringification is a leak surface.
      toastError(pushToast, t("github.loadFailed"), e);
    }
  }, [pushToast, t]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const save = async () => {
    if (!input.trim()) return;
    setBusy(true);
    try {
      // Never retain the raw token in React state; pass once, clear
      // the input, fetch the status back.
      await api.settingsGithubTokenSet(input.trim());
      setInput("");
      await refresh();
      pushToast("info", t("github.saved"));
    } catch (e) {
      toastError(pushToast, t("github.saveFailed"), e);
    } finally {
      setBusy(false);
    }
  };

  const clear = async () => {
    setBusy(true);
    try {
      await api.settingsGithubTokenClear();
      setInput("");
      await refresh();
      pushToast("info", t("github.cleared"));
    } catch (e) {
      toastError(pushToast, t("github.clearFailed"), e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <SettingsGroup desc={t("github.desc")}>
      <Row label={t("github.status")}>
        {status?.present ? (
          <code data-testid="github-token-last4">
            …{status.last4 ?? "????"}
          </code>
        ) : (
          <span style={{ color: "var(--fg-muted)" }}>
            {t("github.noToken")}
          </span>
        )}
      </Row>
      {status?.env_override && (
        <Row label={t("github.override")}>
          <span
            data-testid="github-env-override-note"
            style={{ color: "var(--warn)", fontSize: "var(--fs-xs)" }}
          >
            {t("github.envNote")}
          </span>
        </Row>
      )}
      <Row label={t("github.token")}>
        <input
          type="password"
          aria-label={t("github.tokenAria")}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder="ghp_…"
          style={inputStyle}
          autoComplete="off"
        />
      </Row>
      <div style={actionsStyle}>
        <Button variant="solid" onClick={save} disabled={busy || !input.trim()}>
          {busy
            ? t("shared.saving")
            : status?.present
              ? t("github.replace")
              : t("github.save")}
        </Button>
        {status?.present && (
          <Button variant="ghost" onClick={clear} disabled={busy}>
            {t("github.clear")}
          </Button>
        )}
      </div>
    </SettingsGroup>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                        Shared bits                          */
/* ──────────────────────────────────────────────────────────── */

function SettingsGroup({
  desc,
  children,
}: {
  desc?: string;
  children: React.ReactNode;
}) {
  return (
    <section
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-14)",
        maxWidth: "var(--content-cap-md)",
      }}
    >
      {desc && (
        <p
          style={{
            color: "var(--fg-muted)",
            fontSize: "var(--fs-xs)",
            margin: 0,
            lineHeight: "var(--lh-body)",
          }}
        >
          {desc}
        </p>
      )}
      {children}
    </section>
  );
}

function Row({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "var(--settings-label-col) 1fr",
        gap: "var(--sp-16)",
        alignItems: "start",
        padding: "var(--sp-8) 0",
        borderBottom: "var(--bw-hair) solid var(--line)",
      }}
    >
      <div>
        <div style={{ fontSize: "var(--fs-sm)", color: "var(--fg)" }}>
          {label}
        </div>
        {hint && (
          <div
            style={{
              fontSize: "var(--fs-xs)",
              color: "var(--fg-faint)",
              marginTop: "var(--sp-3)",
              lineHeight: "var(--lh-body)",
            }}
          >
            {hint}
          </div>
        )}
      </div>
      <div style={{ display: "flex", alignItems: "center" }}>{children}</div>
    </div>
  );
}

function Kv({
  label,
  value,
  mono,
  tone,
}: {
  label: string;
  value: React.ReactNode;
  mono?: boolean;
  tone?: "warn";
}) {
  return (
    <>
      <dt
        style={{
          fontSize: "var(--fs-xs)",
          color: "var(--fg-muted)",
          textAlign: "right",
        }}
      >
        {label}
      </dt>
      {/* `.selectable` (base.css), not inline `userSelect: "text"` —
          React omits the -webkit- prefix WKWebView reads first, so
          the inline form never wins over the body opt-out. */}
      <dd
        className="selectable"
        style={{
          margin: 0,
          fontSize: "var(--fs-sm)",
          color: tone === "warn" ? "var(--warn)" : "var(--fg)",
          fontFamily: mono ? "var(--font)" : undefined,
          wordBreak: "break-all",
        }}
      >
        {value}
      </dd>
    </>
  );
}

function Toggle({
  on,
  onChange,
  disabled = false,
}: {
  on: boolean;
  onChange: (next: boolean) => void;
  /**
   * Renders the toggle non-interactive (audit 2026-05 #4: previously
   * accepted from the type position but ignored; clicks still flipped
   * the value). Lets callers honor read-only states such as an env
   * override.
   */
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-disabled={disabled || undefined}
      disabled={disabled}
      onClick={disabled ? undefined : () => onChange(!on)}
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
          borderRadius: "50%",
          background: "var(--bg-raised)",
          boxShadow: "var(--shadow-thumb)",
          transition: "left var(--dur-base) var(--ease-linear)",
        }}
      />
    </button>
  );
}

const inputStyle: React.CSSProperties = {
  height: "var(--row-height)",
  padding: "0 var(--sp-10)",
  fontFamily: "var(--font)",
  fontSize: "var(--fs-sm)",
  color: "var(--fg)",
  background: "var(--bg-raised)",
  border: "var(--bw-hair) solid var(--line)",
  borderRadius: "var(--r-2)",
  outline: "none",
};

const selectStyle: React.CSSProperties = {
  ...inputStyle,
  appearance: "auto",
};

const actionsStyle: React.CSSProperties = {
  display: "flex",
  gap: "var(--sp-8)",
  alignItems: "center",
};

function DisabledReason({ children }: { children: React.ReactNode }) {
  return (
    <span
      style={{
        fontSize: "var(--fs-xs)",
        color: "var(--fg-faint)",
        fontStyle: "italic",
      }}
    >
      {children}
    </span>
  );
}

const gridStyle: React.CSSProperties = {
  display: "grid",
  gridTemplateColumns: "minmax(var(--settings-kv-col), max-content) 1fr",
  columnGap: "var(--sp-16)",
  rowGap: "var(--sp-10)",
  margin: 0,
};
