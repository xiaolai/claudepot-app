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
 * One primary action per view — the split-button. Design.md preserved.
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
  secondaryActions,
  style,
}: PreviewHeaderProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement | null>(null);

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

  const resolvedLabel = useMemo(() => {
    if (!editors || !defaults) return "Open in…";
    const byKindId = kind ? defaults.by_kind[kind] : undefined;
    const pickId = byKindId ?? defaults.fallback ?? "system";
    const picked = editors.find((e) => e.id === pickId);
    if (picked) return `Open in ${picked.label}`;
    const system = editors.find((e) => e.id === "system");
    return system ? `Open in ${system.label}` : "Open in…";
  }, [editors, defaults, kind]);

  const disabled = !path;
  const loading = !editors || !defaults;

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
      <div
        style={{
          display: "flex",
          alignItems: "flex-start",
          justifyContent: "space-between",
          gap: "var(--sp-16)",
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
                fontFamily: "var(--mono)",
                fontSize: "var(--fs-2xs)",
                color: "var(--fg-faint)",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
              title={path}
            >
              {path}
            </div>
          )}
        </div>

        <div
          ref={menuRef}
          style={{ position: "relative", display: "flex", alignItems: "center" }}
        >
          <div
            style={{
              display: "inline-flex",
              alignItems: "stretch",
              borderRadius: "var(--r-2)",
              overflow: "hidden",
              boxShadow: "0 0 0 var(--bw-hair) var(--accent)",
            }}
          >
            <Button
              variant="solid"
              size="md"
              disabled={disabled || loading}
              onClick={() => onOpen(null)}
              aria-label={resolvedLabel}
              style={{ borderRadius: 0, border: "none" }}
            >
              {loading ? "Opening…" : resolvedLabel}
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
                borderLeft: "var(--bw-hair) solid rgba(0,0,0,0.2)",
                cursor: disabled || loading ? "not-allowed" : "pointer",
                opacity: disabled || loading ? "var(--opacity-disabled)" : 1,
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
  onOpen,
  onPickOther,
  onSetDefault,
  onRefreshEditors,
}: {
  editors: EditorCandidateDto[];
  defaults: EditorDefaultsDto;
  kind: ConfigKind | null;
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
        zIndex: 10,
        minWidth: 280,
        maxHeight: "60vh",
        overflowY: "auto",
        background: "var(--bg)",
        border: "var(--bw-hair) solid var(--line-strong)",
        borderRadius: "var(--r-2)",
        boxShadow: "var(--shadow-md)",
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
        style={{
          padding: "var(--sp-6) var(--sp-12) var(--sp-4)",
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-faint)",
          textTransform: "uppercase",
          letterSpacing: "0.05em",
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
        <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis" }}>
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
  const [hover, setHover] = useState(false);
  return (
    <button
      type="button"
      role="menuitem"
      aria-label={ariaLabel}
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      title={title}
      className="pm-focus"
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-8)",
        width: "100%",
        padding: "var(--sp-6) var(--sp-12)",
        background: hover ? "var(--bg-hover)" : "transparent",
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
