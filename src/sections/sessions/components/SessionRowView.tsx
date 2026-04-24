import {
  memo,
  type CSSProperties,
  type MouseEvent,
  useMemo,
  useState,
} from "react";
import { Glyph } from "../../../components/primitives/Glyph";
import { IconButton } from "../../../components/primitives/IconButton";
import { Tag } from "../../../components/primitives/Tag";
import { NF } from "../../../icons";
import { maybeRedact, redactSecrets } from "../../../lib/redactSecrets";
import type { SessionRow } from "../../../types";
import { formatRelativeTime, formatSize } from "../../projects/format";
import {
  bestTimestampMs,
  deriveSessionTitle,
  formatTokens,
  modelBadge,
  projectBasename,
  shortSessionId,
} from "../format";
import { COLS } from "../sessionsTable.shared";

interface SessionRowProps {
  session: SessionRow;
  active: boolean;
  onSelect: (filePath: string) => void;
  onContextMenu?: (e: MouseEvent, s: SessionRow) => void;
  snippet?: string;
  /** When rendered under the virtualizer, `virtualStart` is the pixel
   * offset this row translates to. Passed as a primitive so
   * `React.memo`'s shallow equality works — a style object literal
   * here would change identity every render. */
  virtualStart?: number;
  /** Virtualizer's measurement callback — must be attached as a ref so
   * the library records each row's real height on paint. */
  measureRef?: (el: HTMLElement | null) => void;
  /** Index in the virtualized sequence. Required by `measureElement`
   * to reconcile the measured node back to its virtual row. Also
   * exposed as `aria-posinset` so screen readers say "row X of Y"
   * even when the DOM only carries the viewport's worth of items. */
  virtualIndex?: number;
  /** Total option count for the listbox — emitted as `aria-setsize`
   * on virtualized rows so assistive tech sees the full set, not
   * just the mounted window. Plain-path rows leave this undefined
   * since the DOM already carries every option. */
  virtualSetSize?: number;
}

/**
 * One row in the Sessions table. Memoized so that unrelated parent
 * state changes (toast, ctx menu, refresh spinner) don't re-render
 * the visible row set. All props are either primitives or
 * referentially stable (Map, callbacks via state setter / useCallback)
 * so shallow equality holds in practice.
 */
export const SessionRowView = memo(function SessionRowView({
  session: s,
  active,
  onSelect,
  onContextMenu,
  snippet,
  virtualStart,
  measureRef,
  virtualIndex,
  virtualSetSize,
}: SessionRowProps) {
  const [hover, setHover] = useState(false);
  const lastTs = bestTimestampMs(s.last_ts, s.last_modified_ms);
  // Defense in depth: even though the Rust side redacts
  // `first_user_prompt` at codec.rs::upsert_row, every render of a
  // user-controlled string runs through `redactSecrets` so a backend
  // regression can't surface a raw token in the DOM. Memoized per
  // row so we only pay the scan when the row's identity changes.
  // Other user-controllable fields (`project_path`, `git_branch`,
  // models) are redacted at their render sites below — they aren't
  // run through the Rust redactor at upsert.
  const safePrompt = useMemo(
    () => maybeRedact(s.first_user_prompt),
    [s.first_user_prompt],
  );
  const cleanTitle = useMemo(
    () => deriveSessionTitle(safePrompt),
    [safePrompt],
  );
  const project = redactSecrets(projectBasename(s.project_path) || s.slug);
  const projectTitle = useMemo(
    () => redactSecrets(s.project_path),
    [s.project_path],
  );
  const safeBranch = useMemo(() => maybeRedact(s.git_branch), [s.git_branch]);
  const headline =
    cleanTitle ?? (s.is_sidechain ? "Agent subsession" : shortSessionId(s.session_id));
  const model = modelBadge(s.models);
  const tokens = formatTokens(s.tokens.total);

  // `virtualStart` decides whether this row is rendered under the
  // virtualizer. When it is, the style object is constructed here
  // from primitives so it can be recomputed cheaply when needed and
  // referentially stable when unchanged.
  const virtualStyle: CSSProperties | undefined =
    virtualStart !== undefined
      ? {
          position: "absolute",
          top: 0,
          left: 0,
          width: "100%",
          transform: `translateY(${virtualStart}px)`,
        }
      : undefined;

  return (
    <li
      ref={measureRef}
      data-index={virtualIndex}
      role="option"
      aria-selected={active}
      aria-posinset={
        virtualIndex !== undefined ? virtualIndex + 1 : undefined
      }
      aria-setsize={virtualSetSize}
      tabIndex={0}
      onClick={() => onSelect(s.file_path)}
      onContextMenu={onContextMenu ? (e) => onContextMenu(e, s) : undefined}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect(s.file_path);
        }
      }}
      style={{
        display: "grid",
        gridTemplateColumns: COLS,
        padding: "var(--sp-12) var(--sp-32)",
        gap: "var(--sp-16)",
        alignItems: "center",
        borderBottom: "var(--bw-hair) solid var(--line)",
        background: active
          ? "var(--bg-active)"
          : hover
            ? "var(--bg-hover)"
            : "transparent",
        borderLeft: active
          ? "var(--bw-strong) solid var(--accent)"
          : "var(--bw-strong) solid transparent",
        cursor: "pointer",
        fontSize: "var(--fs-sm)",
        outline: "none",
        ...virtualStyle,
      }}
    >
      <span aria-hidden>
        <Glyph
          g={s.has_error ? NF.warn : NF.chatAlt}
          color={s.has_error ? "var(--warn)" : "var(--fg-muted)"}
          style={{ fontSize: "var(--fs-md)" }}
        />
      </span>

      <div style={{ minWidth: 0, overflow: "hidden" }}>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--sp-8)",
            fontSize: "var(--fs-base)",
            color: "var(--fg)",
            fontWeight: active ? 600 : 500,
            minWidth: 0,
          }}
        >
          <span
            title={safePrompt ?? s.session_id}
            style={{
              whiteSpace: "nowrap",
              overflow: "hidden",
              textOverflow: "ellipsis",
            }}
          >
            {headline}
          </span>
          {s.is_sidechain && (
            <Tag tone="ghost" title="Agent subsession">
              agent
            </Tag>
          )}
          {s.has_error && (
            <Tag tone="warn" glyph={NF.warn} title="This session had an error">
              error
            </Tag>
          )}
        </div>
        <div
          style={{
            marginTop: "var(--sp-2)",
            color: "var(--fg-faint)",
            fontSize: "var(--fs-xs)",
            display: "block",
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
          }}
        >
          {(() => {
            // Build the meta line as one inline text run so that when
            // overflow truncates the right edge, the separator that
            // would otherwise precede a clipped value never stays
            // orphaned on screen. Each segment carries its own leading
            // separator; ellipsis then cuts cleanly through the value
            // it belongs to.
            const parts: Array<{ key: string; node: React.ReactNode }> = [];
            parts.push({
              key: "id",
              node: (
                <span className="mono">{shortSessionId(s.session_id)}</span>
              ),
            });
            if (model) parts.push({ key: "model", node: <span>{model}</span> });
            if (safeBranch) {
              parts.push({
                key: "branch",
                node: (
                  <span
                    style={{
                      display: "inline-flex",
                      alignItems: "baseline",
                      gap: "var(--sp-4)",
                    }}
                  >
                    <Glyph g={NF.branch} style={{ fontSize: "var(--fs-2xs)" }} />
                    {safeBranch}
                  </span>
                ),
              });
            }
            if (s.file_size_bytes > 0) {
              parts.push({
                key: "size",
                node: <span>{formatSize(s.file_size_bytes)}</span>,
              });
            }
            return parts.map((p, i) => (
              <span key={p.key}>
                {i > 0 && (
                  <span style={{ margin: "0 var(--sp-8)" }} aria-hidden>
                    ·
                  </span>
                )}
                {p.node}
              </span>
            ));
          })()}
        </div>
        {snippet && (() => {
          // Defense in depth: backend already redacts via
          // session_search::make_hit, but if a future regression ever
          // bypassed that, the UI must not surface the raw token.
          const safeSnippet = redactSecrets(snippet);
          return (
            <div
              data-testid="search-snippet"
              title={safeSnippet}
              style={{
                marginTop: "var(--sp-4)",
                color: "var(--fg-muted)",
                fontSize: "var(--fs-xs)",
                whiteSpace: "nowrap",
                overflow: "hidden",
                textOverflow: "ellipsis",
                fontStyle: "italic",
              }}
            >
              {safeSnippet}
            </div>
          );
        })()}
      </div>

      <div style={{ minWidth: 0, overflow: "hidden" }}>
        <div
          title={projectTitle}
          style={{
            color: "var(--fg-muted)",
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
          }}
        >
          {project}
        </div>
        {!s.project_from_transcript && (
          <div
            style={{
              marginTop: "var(--sp-2)",
              color: "var(--fg-ghost)",
              fontSize: "var(--fs-xs)",
            }}
            title="Decoded from the on-disk slug — the transcript didn't carry a cwd field"
          >
            decoded from slug
          </div>
        )}
      </div>

      <span
        style={{
          color: s.message_count > 0 ? "var(--fg-muted)" : "var(--fg-ghost)",
          fontVariantNumeric: "tabular-nums",
        }}
        title={`${s.user_message_count} user · ${s.assistant_message_count} assistant`}
      >
        {s.message_count > 0 ? s.message_count : "—"}
      </span>

      <span
        style={{
          color: s.tokens.total > 0 ? "var(--fg-muted)" : "var(--fg-ghost)",
          fontVariantNumeric: "tabular-nums",
        }}
        title={
          s.tokens.total > 0
            ? `input ${s.tokens.input} · output ${s.tokens.output} · cache r/w ${s.tokens.cache_read}/${s.tokens.cache_creation}`
            : undefined
        }
      >
        {tokens || "—"}
      </span>

      <span
        style={{
          color: "var(--fg-faint)",
          whiteSpace: "nowrap",
          overflow: "hidden",
          textOverflow: "ellipsis",
        }}
      >
        {lastTs != null ? formatRelativeTime(lastTs) : "—"}
      </span>

      <span
        style={{
          display: "inline-flex",
          alignItems: "center",
          gap: "var(--sp-2)",
          justifyContent: "flex-end",
        }}
      >
        {(hover || active) && onContextMenu && (
          <span
            onClick={(e) => e.stopPropagation()}
            onMouseDown={(e) => e.stopPropagation()}
            style={{ display: "inline-flex" }}
          >
            <IconButton
              glyph={NF.ellipsis}
              size="sm"
              onClick={() => {
                const el = document.activeElement as HTMLElement | null;
                const rect = el?.getBoundingClientRect();
                onContextMenu(
                  {
                    preventDefault: () => {},
                    stopPropagation: () => {},
                    clientX: rect ? rect.right : 0,
                    clientY: rect ? rect.bottom : 0,
                  } as unknown as MouseEvent,
                  s,
                );
              }}
              title="More actions"
              aria-label={`More actions for this session`}
              aria-haspopup="menu"
            />
          </span>
        )}
        {(hover || active) && (
          <Glyph
            g={NF.chevronR}
            color={active ? "var(--accent)" : "var(--fg-faint)"}
            style={{ fontSize: "var(--fs-xs)" }}
          />
        )}
      </span>
    </li>
  );
});
