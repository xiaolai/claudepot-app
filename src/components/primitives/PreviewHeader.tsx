import {
  type CSSProperties,
  type ReactNode,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Glyph } from "./Glyph";
import { NF } from "../../icons";
import { Button } from "./Button";
import { BackAffordance } from "./BackAffordance";
import type {
  ConfigKind,
  EditorCandidateDto,
  EditorDefaultsDto,
} from "../../types";

interface PreviewHeaderProps {
  title: string;
  subtitle?: ReactNode;
  /** File path (for display + launch). */
  path: string | null;
  kind?: ConfigKind | null;
  /** Detected editor list. `null` means "loading". */
  editors: EditorCandidateDto[] | null;
  /** Persisted defaults — per-kind + fallback. */
  defaults: EditorDefaultsDto | null;
  onOpen: (editorId: string | null) => void;
  onPickOther: () => void;
  onSetDefault: (kind: ConfigKind | null, editorId: string) => void;
  onRefreshEditors: () => void;
  /**
   * Closes this preview and returns the right pane to its empty/home
   * state. Renders a small chevron-left button above the title when
   * set. Omit to suppress the affordance — used by `ConfigHomePane`,
   * which IS the home view and has nowhere to go back to.
   */
  onClose?: () => void;
  /** Secondary (kebab) actions. */
  secondaryActions?: ReactNode;
  style?: CSSProperties;
}

/**
 * Preview-header split-button per `dev-docs/config-section-plan.md`
 * §13.4: left half launches the resolved default editor for this
 * Kind; right half opens a menu listing every detected editor +
 * `$EDITOR` + system default + "Other…" + "Set as default for
 * <Kind>" / "Set as fallback default" + "Refresh editor list".
 *
 * One primary action per view — the split-button. design.md preserved.
 *
 * The split-button group caps its label at `--config-cmd-col-max`
 * with ellipsis, so long editor names ("Visual Studio Code") never
 * push the group beyond its grid column.
 */
export function PreviewHeader({
  title,
  subtitle,
  path,
  kind = null,
  editors,
  defaults,
  onOpen,
  onPickOther,
  onSetDefault,
  onRefreshEditors,
  onClose,
  secondaryActions,
  style,
}: PreviewHeaderProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const [menuMaxHeight, setMenuMaxHeight] = useState<string>(
    "var(--config-menu-max-height)",
  );
  const menuRef = useRef<HTMLDivElement | null>(null);
  const splitRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!menuOpen) return;
    const onDocClick = (e: MouseEvent) => {
      if (!menuRef.current) return;
      if (menuRef.current.contains(e.target as Node)) return;
      setMenuOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setMenuOpen(false);
    };
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onKey);
    };
  }, [menuOpen]);

  // When the menu opens, measure the anchor against the viewport so we
  // can cap max-height at (viewport-bottom - anchor-bottom - padding).
  // Falls back to the token ceiling when the measurement would leave
  // too little room.
  useEffect(() => {
    if (!menuOpen || !splitRef.current) return;
    const r = splitRef.current.getBoundingClientRect();
    const available = window.innerHeight - r.bottom - 24;
    if (available < 200) {
      setMenuMaxHeight("var(--config-menu-max-height)");
      return;
    }
    setMenuMaxHeight(`min(var(--config-menu-max-height), ${available}px)`);
  }, [menuOpen]);

  // Resolve the editor the primary click would launch. Returns the
  // editor name only — the "Open in …" verb is carried by the glyph
  // (NF.openExternal) so the button can render as `↗ VS Code` instead
  // of `Open in VS Code`, saving ~7 chars of horizontal space without
  // losing the "which editor" signal.
  const resolvedEditor = useMemo(() => {
    if (!editors || !defaults) return null;
    const byKindId = kind ? defaults.by_kind[kind] : undefined;
    const pickId = byKindId ?? defaults.fallback ?? "system";
    return (
      editors.find((e) => e.id === pickId) ??
      editors.find((e) => e.id === "system") ??
      null
    );
  }, [editors, defaults, kind]);

  const disabled = !path;
  const loading = !editors || !defaults;
  // Glyph only shows next to an actual editor name — the
  // loading / no-editor placeholders carry the meaning in words
  // alone, so a bare arrow would leave the user guessing.
  const showGlyph = !loading && resolvedEditor != null;
  const buttonLabel = loading
    ? "Detecting editors…"
    : resolvedEditor
      ? resolvedEditor.label
      : "Open in…";
  // `aria-label` and hover title keep the full "Open in <Editor>" for
  // screen readers and tooltip users — the visible text is just the
  // editor name.
  const buttonAccessibleLabel = loading
    ? "Detecting editors…"
    : resolvedEditor
      ? `Open in ${resolvedEditor.label}`
      : "Open in…";

  return (
    <header
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-6)",
        padding: "var(--sp-16) var(--sp-20)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        ...style,
      }}
    >
      {onClose && (
        <BackAffordance
          label="Artifacts"
          onClick={onClose}
          title="Back to artifact list"
          style={{ marginBottom: "var(--sp-2)" }}
        />
      )}
      <div
        style={{
          display: "flex",
          alignItems: "flex-start",
          justifyContent: "space-between",
          gap: "var(--sp-16)",
          minWidth: 0,
        }}
      >
        <div style={{ minWidth: 0, flex: 1 }}>
          <h2
            style={{
              margin: 0,
              fontSize: "var(--fs-lg)",
              fontWeight: 600,
              color: "var(--fg)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {title}
          </h2>
          {subtitle && (
            <div
              style={{
                marginTop: "var(--sp-4)",
                fontSize: "var(--fs-xs)",
                color: "var(--fg-faint)",
              }}
            >
              {subtitle}
            </div>
          )}
          {path && (
            <div
              style={{
                marginTop: "var(--sp-6)",
                fontFamily: "var(--font-mono)",
                fontSize: "var(--fs-2xs)",
                color: "var(--fg-faint)",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
                direction: "rtl",
                textAlign: "left",
              }}
              title={path}
            >
              {/* dir="ltr" wrapper preserves the leading slash sign while
                   direction:rtl causes ellipsis to bite off the LEFT
                   (start) of the path — so the filename stays visible
                   when the column is narrow. */}
              <bdi dir="ltr">{path}</bdi>
            </div>
          )}
        </div>

        <div
          ref={menuRef}
          style={{
            position: "relative",
            display: "flex",
            alignItems: "center",
            flexShrink: 0,
          }}
        >
          <div
            ref={splitRef}
            style={{
              display: "inline-flex",
              alignItems: "stretch",
              borderRadius: "var(--r-2)",
              overflow: "hidden",
              boxShadow: "0 0 0 var(--bw-hair) var(--accent)",
              maxWidth: "var(--config-cmd-col-max)",
            }}
          >
            <Button
              variant="solid"
              size="md"
              disabled={disabled || loading}
              onClick={() => onOpen(null)}
              aria-label={buttonAccessibleLabel}
              glyph={showGlyph ? NF.openExternal : undefined}
              style={{
                borderRadius: 0,
                border: "none",
                minWidth: 0,
                overflow: "hidden",
                // `flex` (not `block`) so the glyph sits next to the
                // label instead of pushing it below.
                display: "flex",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
              title={buttonAccessibleLabel}
            >
              {buttonLabel}
            </Button>
            <button
              type="button"
              disabled={disabled || loading}
              aria-haspopup="menu"
              aria-expanded={menuOpen}
              aria-label="Choose editor"
              onClick={() => setMenuOpen((v) => !v)}
              className="pm-focus"
              style={{
                display: "inline-flex",
                alignItems: "center",
                justifyContent: "center",
                padding: "0 var(--sp-6)",
                background: "var(--accent)",
                color: "var(--on-color)",
                border: "none",
                borderLeft:
                  "var(--bw-hair) solid var(--accent-divider-ink)",
                cursor: disabled || loading ? "not-allowed" : "pointer",
                opacity: disabled || loading ? "var(--opacity-disabled)" : 1,
                flexShrink: 0,
              }}
            >
              <Glyph g={NF.chevronD} />
            </button>
          </div>

          {menuOpen && editors && defaults && (
            <EditorMenu
              editors={editors}
              defaults={defaults}
              kind={kind}
              maxHeight={menuMaxHeight}
              onOpen={(editorId) => {
                setMenuOpen(false);
                onOpen(editorId);
              }}
              onPickOther={() => {
                setMenuOpen(false);
                onPickOther();
              }}
              onSetDefault={(k, id) => {
                setMenuOpen(false);
                onSetDefault(k, id);
              }}
              onRefreshEditors={() => {
                setMenuOpen(false);
                onRefreshEditors();
              }}
            />
          )}
        </div>
      </div>

      {secondaryActions && (
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--sp-6)",
            flexWrap: "wrap",
          }}
        >
          {secondaryActions}
        </div>
      )}
    </header>
  );
}

function EditorMenu({
  editors,
  defaults,
  kind,
  maxHeight,
  onOpen,
  onPickOther,
  onSetDefault,
  onRefreshEditors,
}: {
  editors: EditorCandidateDto[];
  defaults: EditorDefaultsDto;
  kind: ConfigKind | null;
  maxHeight: string;
  onOpen: (editorId: string) => void;
  onPickOther: () => void;
  onSetDefault: (kind: ConfigKind | null, editorId: string) => void;
  onRefreshEditors: () => void;
}) {
  const detected = editors.filter(
    (e) => e.id !== "env" && e.id !== "system",
  );
  const env = editors.find((e) => e.id === "env");
  const system = editors.find((e) => e.id === "system");

  return (
    <div
      role="menu"
      aria-label="Open with"
      style={{
        position: "absolute",
        top: "calc(100% + var(--sp-4))",
        right: 0,
        zIndex: "var(--z-popover)" as unknown as number,
        minWidth: "var(--config-menu-min-width)",
        maxHeight,
        overflowY: "auto",
        background: "var(--bg-raised)",
        border: "var(--bw-hair) solid var(--line-strong)",
        borderRadius: "var(--r-2)",
        boxShadow: "var(--shadow-popover)",
        padding: "var(--sp-4) 0",
      }}
    >
      {detected.length > 0 && (
        <MenuGroup label="Detected">
          {detected.map((c) => (
            <EditorRow
              key={c.id}
              candidate={c}
              kind={kind}
              onOpen={onOpen}
              onSetDefault={onSetDefault}
            />
          ))}
        </MenuGroup>
      )}
      {env && (
        <MenuGroup label="$EDITOR">
          <EditorRow
            candidate={env}
            kind={kind}
            onOpen={onOpen}
            onSetDefault={onSetDefault}
          />
        </MenuGroup>
      )}
      {system && (
        <MenuGroup label="System default">
          <EditorRow
            candidate={system}
            kind={kind}
            onOpen={onOpen}
            onSetDefault={onSetDefault}
          />
        </MenuGroup>
      )}
      <div
        role="separator"
        aria-orientation="horizontal"
        style={{
          height: "var(--bw-hair)",
          background: "var(--line)",
          margin: "var(--sp-4) 0",
        }}
      />
      <MenuItem onClick={onPickOther}>
        <Glyph g={NF.ellipsis} color="var(--fg-muted)" />
        <span>Other…</span>
      </MenuItem>
      <MenuItem onClick={onRefreshEditors}>
        <Glyph g={NF.refresh} color="var(--fg-muted)" />
        <span>Refresh editor list</span>
      </MenuItem>
      <div
        style={{
          padding: "var(--sp-6) var(--sp-12)",
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-faint)",
        }}
      >
        Fallback: <strong>{defaults.fallback}</strong>
      </div>
    </div>
  );
}

function MenuGroup({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <div>
      <div
        className="mono-cap"
        style={{
          padding: "var(--sp-6) var(--sp-12) var(--sp-4)",
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-faint)",
          textTransform: "uppercase",
          letterSpacing: "var(--ls-wide)",
        }}
      >
        {label}
      </div>
      {children}
    </div>
  );
}

function EditorRow({
  candidate,
  kind,
  onOpen,
  onSetDefault,
}: {
  candidate: EditorCandidateDto;
  kind: ConfigKind | null;
  onOpen: (editorId: string) => void;
  onSetDefault: (kind: ConfigKind | null, editorId: string) => void;
}) {
  return (
    <div style={{ display: "flex", alignItems: "stretch" }}>
      <MenuItem onClick={() => onOpen(candidate.id)} style={{ flex: 1 }}>
        <span
          style={{
            flex: 1,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {candidate.label}
        </span>
      </MenuItem>
      {kind && (
        <MenuItem
          onClick={() => onSetDefault(kind, candidate.id)}
          title={`Set ${candidate.label} as default for ${kind}`}
          style={{ flex: "0 0 auto" }}
          aria-label={`Set default for ${kind}`}
        >
          <span
            style={{
              fontSize: "var(--fs-2xs)",
              color: "var(--fg-faint)",
            }}
          >
            set default
          </span>
        </MenuItem>
      )}
      <MenuItem
        onClick={() => onSetDefault(null, candidate.id)}
        title={`Set ${candidate.label} as fallback default`}
        style={{ flex: "0 0 auto" }}
        aria-label="Set fallback default"
      >
        <span
          style={{
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-faint)",
          }}
        >
          set fallback
        </span>
      </MenuItem>
    </div>
  );
}

/**
 * Menu row with CSS-based hover — no per-row useState. Paper-mono
 * convention uses the shared `.pm-menu-item` class (declared in
 * `App.css`) to render the hover background via pseudo-class.
 */
function MenuItem({
  children,
  onClick,
  title,
  style,
  "aria-label": ariaLabel,
}: {
  children: ReactNode;
  onClick: () => void;
  title?: string;
  style?: CSSProperties;
  "aria-label"?: string;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      aria-label={ariaLabel}
      onClick={onClick}
      title={title}
      className="pm-focus pm-menu-item"
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-8)",
        width: "100%",
        padding: "var(--sp-6) var(--sp-12)",
        background: "transparent",
        color: "var(--fg)",
        border: "none",
        textAlign: "left",
        fontSize: "var(--fs-sm)",
        cursor: "pointer",
        ...style,
      }}
    >
      {children}
    </button>
  );
}
