import { Button } from "../../components/primitives/Button";
import { IconButton } from "../../components/primitives/IconButton";
import { Tag } from "../../components/primitives/Tag";
import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";
import type { RouteSummaryDto } from "../../types";

interface RouteCardProps {
  route: RouteSummaryDto;
  busy: boolean;
  onUseCli: (id: string) => void;
  onUnuseCli: (id: string) => void;
  onUseDesktop: (id: string) => void;
  onUnuseDesktop: (id: string) => void;
  onRemove: (id: string) => void;
  onEdit: (route: RouteSummaryDto) => void;
}

export function RouteCard({
  route,
  busy,
  onUseCli,
  onUnuseCli,
  onUseDesktop,
  onUnuseDesktop,
  onRemove,
  onEdit,
}: RouteCardProps) {
  return (
    <article
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-12)",
        padding: "var(--sp-16)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-3)",
        background: "var(--bg-raised)",
      }}
    >
      <header
        style={{
          display: "flex",
          alignItems: "baseline",
          gap: "var(--sp-8)",
          flexWrap: "wrap",
        }}
      >
        <h3
          style={{
            margin: 0,
            fontSize: "var(--fs-md)",
            color: "var(--fg-strong)",
            fontWeight: 600,
          }}
        >
          {route.name}
        </h3>
        <Tag tone="neutral">{route.provider_kind}</Tag>
        {route.use_keychain && (
          <Tag
            tone="ok"
            glyph={NF.lock}
            title="Secret is held in the OS keychain; the wrapper + Cowork helper read it on demand."
          >
            Keychain
          </Tag>
        )}
        {route.active_on_desktop && (
          <Tag tone="accent" title="Mirrored into Claude Desktop's enterpriseConfig">
            Active on Desktop
          </Tag>
        )}
      </header>

      <dl
        style={{
          display: "grid",
          gridTemplateColumns: "auto 1fr",
          columnGap: "var(--sp-12)",
          rowGap: "var(--sp-4)",
          margin: 0,
          fontSize: "var(--fs-sm)",
        }}
      >
        <dt style={{ color: "var(--fg-faint)" }}>Endpoint</dt>
        <dd
          style={{
            margin: 0,
            color: "var(--fg)",
            overflow: "hidden",
            textOverflow: "ellipsis",
          }}
        >
          {route.base_url}
        </dd>

        <dt style={{ color: "var(--fg-faint)" }}>Key</dt>
        <dd style={{ margin: 0, color: "var(--fg)" }}>
          <code>{route.api_key_preview}</code>
        </dd>

        <dt style={{ color: "var(--fg-faint)" }}>Model</dt>
        <dd style={{ margin: 0, color: "var(--fg)" }}>
          <code>{route.model}</code>
          {route.additional_models.length > 0 && (
            <span style={{ color: "var(--fg-faint)" }}>
              {" "}
              + {route.additional_models.length} more
            </span>
          )}
        </dd>

        <dt style={{ color: "var(--fg-faint)" }}>Wrapper</dt>
        <dd style={{ margin: 0, color: "var(--fg)" }}>
          <code>{route.wrapper_name}</code>
          {route.installed_on_cli ? (
            <span
              style={{
                marginLeft: "var(--sp-8)",
                color: "var(--fg-faint)",
              }}
            >
              <Glyph g={NF.check} /> on PATH
            </span>
          ) : (
            <span
              style={{
                marginLeft: "var(--sp-8)",
                color: "var(--fg-faint)",
              }}
            >
              not installed
            </span>
          )}
        </dd>
      </dl>

      <footer
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "center",
          gap: "var(--sp-8)",
          marginTop: "var(--sp-4)",
        }}
      >
        <div style={{ display: "flex", gap: "var(--sp-8)", flexWrap: "wrap" }}>
          {route.installed_on_cli ? (
            <Button
              variant="outline"
              size="sm"
              onClick={() => onUnuseCli(route.id)}
              disabled={busy}
              glyph={NF.minus}
              title="Delete the wrapper script from ~/.claudepot/bin/"
            >
              Uninstall CLI
            </Button>
          ) : (
            <Button
              variant="outline"
              size="sm"
              onClick={() => onUseCli(route.id)}
              disabled={busy}
              glyph={NF.terminal}
              title="Write the wrapper script to ~/.claudepot/bin/"
            >
              Use in CLI
            </Button>
          )}
          {route.active_on_desktop ? (
            <Button
              variant="outline"
              size="sm"
              onClick={() => onUnuseDesktop(route.id)}
              disabled={busy}
              glyph={NF.minus}
              title="Clear enterpriseConfig (the Desktop profile stays defined)"
            >
              Deactivate Desktop
            </Button>
          ) : (
            <Button
              variant="outline"
              size="sm"
              onClick={() => onUseDesktop(route.id)}
              disabled={busy}
              glyph={NF.desktop}
              title="Mirror this route into Claude Desktop's enterpriseConfig"
            >
              Use in Desktop
            </Button>
          )}
        </div>
        <div style={{ display: "flex", gap: "var(--sp-4)" }}>
          <IconButton
            glyph={NF.edit}
            onClick={() => onEdit(route)}
            disabled={busy}
            title="Edit this route"
            aria-label="Edit route"
          />
          <IconButton
            glyph={NF.trash}
            onClick={() => onRemove(route.id)}
            disabled={busy}
            title="Delete this route — also tears down its CLI wrapper and Desktop activation"
            aria-label="Delete route"
          />
        </div>
      </footer>
    </article>
  );
}
