import { type ReactNode, useMemo, useState } from "react";
import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";

/**
 * Collapsible JSON tree — renders the parsed body of settings.json,
 * settings.local.json, managed-settings.json, mcp.json, plugin
 * manifests, and keybindings.json. When the body can't be parsed
 * (malformed), falls back to a plain preformatted view with a warning
 * banner so the user still sees something useful.
 *
 * The input body has already been secret-masked upstream by
 * `mask_bytes` in core — rendering is display-only.
 */
export function JsonTreeRenderer({ body }: { body: string }) {
  const parsed = useMemo(() => tryParse(body), [body]);

  if (parsed.kind === "error") {
    return (
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "var(--sp-8)",
          padding: "var(--sp-16) var(--sp-20)",
        }}
      >
        <div
          role="status"
          style={{
            display: "inline-flex",
            alignItems: "center",
            gap: "var(--sp-6)",
            padding: "var(--sp-6) var(--sp-10)",
            border: "var(--bw-hair) solid var(--line)",
            background: "var(--bg-sunken)",
            color: "var(--danger)",
            fontSize: "var(--fs-xs)",
            borderRadius: "var(--r-2)",
            alignSelf: "flex-start",
          }}
        >
          <Glyph g={NF.warn} color="var(--danger)" />
          <span>JSON parse failed: {parsed.message}</span>
        </div>
        <pre
          style={{
            margin: 0,
            fontFamily: "var(--font-mono)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg)",
            whiteSpace: "pre-wrap",
            overflowWrap: "anywhere",
          }}
        >
          {body}
        </pre>
      </div>
    );
  }

  return (
    <div
      style={{
        padding: "var(--sp-12) var(--sp-16)",
        fontFamily: "var(--font-mono)",
        fontSize: "var(--fs-xs)",
        color: "var(--fg)",
        overflowWrap: "anywhere",
      }}
    >
      <Node value={parsed.value} depth={0} initiallyOpen={true} />
    </div>
  );
}

type ParseResult =
  | { kind: "ok"; value: unknown }
  | { kind: "error"; message: string };

function tryParse(body: string): ParseResult {
  try {
    return { kind: "ok", value: JSON.parse(body) };
  } catch (e) {
    return { kind: "error", message: e instanceof Error ? e.message : String(e) };
  }
}

// Hard cap on tree depth. Adversarial JSON (e.g. a million-deep array
// chain) would otherwise blow the React render stack and freeze the
// pane. 64 is well past anything CC writes in practice (audit
// 2026-04-24, T3 H2).
const MAX_DEPTH = 64;

function Node({
  value,
  depth,
  initiallyOpen = false,
  label,
}: {
  value: unknown;
  depth: number;
  initiallyOpen?: boolean;
  label?: string;
}): ReactNode {
  // Primitive leaf
  if (
    value === null ||
    typeof value !== "object"
  ) {
    return (
      <Line depth={depth}>
        {label && <Key k={label} />}
        <Value value={value} />
      </Line>
    );
  }

  // Depth cap — past the limit, render the container as a sealed
  // marker rather than recursing further. Keeps the tree useful for
  // the prefix the user can actually navigate to.
  if (depth >= MAX_DEPTH) {
    return (
      <Line depth={depth}>
        {label && <Key k={label} />}
        <span style={{ color: "var(--fg-faint)" }}>
          {Array.isArray(value) ? "[…]" : "{…}"} (max depth)
        </span>
      </Line>
    );
  }

  if (Array.isArray(value)) {
    return (
      <Collapsible
        depth={depth}
        label={label}
        openBracket="["
        closeBracket="]"
        count={value.length}
        initiallyOpen={initiallyOpen || depth < 1}
        renderChildren={() =>
          value.map((item, i) => (
            <Node key={i} value={item} depth={depth + 1} label={undefined} />
          ))
        }
      />
    );
  }

  const obj = value as Record<string, unknown>;
  const keys = Object.keys(obj);
  return (
    <Collapsible
      depth={depth}
      label={label}
      openBracket="{"
      closeBracket="}"
      count={keys.length}
      initiallyOpen={initiallyOpen || depth < 1}
      renderChildren={() =>
        keys.map((k) => (
          <Node key={k} value={obj[k]} depth={depth + 1} label={k} />
        ))
      }
    />
  );
}

function Collapsible({
  depth,
  label,
  openBracket,
  closeBracket,
  count,
  initiallyOpen,
  renderChildren,
}: {
  depth: number;
  label: string | undefined;
  openBracket: "{" | "[";
  closeBracket: "}" | "]";
  count: number;
  initiallyOpen: boolean;
  // Lazily produces children only when the node is open. Collapsed
  // arrays/objects must NOT recurse into their contents — large JSON
  // bodies would otherwise build every descendant `Node` upfront and
  // freeze the pane (audit 2026-04-24, T3 H2).
  renderChildren: () => ReactNode;
}) {
  const [open, setOpen] = useState(initiallyOpen);
  return (
    <div>
      <Line depth={depth}>
        <button
          type="button"
          aria-expanded={open}
          onClick={() => setOpen((v) => !v)}
          className="pm-focus"
          style={{
            display: "inline-flex",
            alignItems: "center",
            gap: "var(--sp-3)",
            background: "transparent",
            border: "none",
            padding: 0,
            color: "inherit",
            cursor: "pointer",
            font: "inherit",
          }}
        >
          <Glyph
            g={open ? NF.chevronD : NF.chevronR}
            color="var(--fg-muted)"
          />
          {label && <Key k={label} />}
          <span style={{ color: "var(--fg-muted)" }}>{openBracket}</span>
          {!open && (
            <span style={{ color: "var(--fg-faint)" }}>
              {count} {count === 1 ? "entry" : "entries"}
            </span>
          )}
          {!open && (
            <span style={{ color: "var(--fg-muted)" }}>{closeBracket}</span>
          )}
        </button>
      </Line>
      {open && (
        <>
          {renderChildren()}
          <Line depth={depth}>
            <span style={{ color: "var(--fg-muted)" }}>{closeBracket}</span>
          </Line>
        </>
      )}
    </div>
  );
}

function Line({ depth, children }: { depth: number; children: ReactNode }) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "flex-start",
        gap: "var(--sp-4)",
        paddingLeft: `calc(var(--sp-14) * ${depth})`,
        minHeight: "1.4em",
      }}
    >
      {children}
    </div>
  );
}

function Key({ k }: { k: string }) {
  return (
    <>
      <span style={{ color: "var(--accent-ink)" }}>
        "{k}"
      </span>
      <span style={{ color: "var(--fg-muted)" }}>:</span>
    </>
  );
}

function Value({ value }: { value: unknown }) {
  if (value === null) {
    return <span style={{ color: "var(--fg-faint)" }}>null</span>;
  }
  if (typeof value === "string") {
    return (
      <span
        style={{
          color: "var(--fg)",
          overflowWrap: "anywhere",
          wordBreak: "break-word",
        }}
      >
        "{value}"
      </span>
    );
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return (
      <span style={{ color: "var(--fg)" }}>
        {String(value)}
      </span>
    );
  }
  return null;
}
