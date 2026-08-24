import { useEffect, useId, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { Button } from "../../components/primitives/Button";
import { Input } from "../../components/primitives/Input";
import { FieldBlock } from "../../components/primitives/modalParts";
import { NF } from "../../icons";
import { api } from "../../api";
import { readFromNetworkPanelBreadcrumb } from "../../lib/networkPanelDeepLink";
import type {
  BedrockInputDto,
  FoundryInputDto,
  GatewayInputDto,
  RouteCreateDto,
  RouteDetailsDto,
  RouteProviderKind,
  RouteUpdateDto,
  VertexInputDto,
} from "../../types";

export interface RouteFormProps {
  mode: "add" | "edit";
  /**
   * Pre-population for edit mode. Carries every provider-specific
   * non-secret field. Secrets stay opaque (`*_preview`,
   * `has_*`) — the form leaves the secret input blank and the
   * Rust-side policy is "blank = keep existing".
   */
  initial?: RouteDetailsDto | null;
  onSubmit: (
    payload: RouteCreateDto | RouteUpdateDto,
  ) => Promise<void>;
  onCancel: () => void;
}

/**
 * Tab order. The visible label + subtitle for each kind are NOT
 * stored here: a module-level constant is evaluated once at import,
 * so a label baked in at that moment would keep rendering the boot
 * language after a live switch. Both are resolved per render from
 * `providerTabs.*` via the maps below.
 */
const PROVIDER_KINDS: RouteProviderKind[] = [
  "gateway",
  "bedrock",
  "vertex",
  "foundry",
];

const PROVIDER_LABEL_KEYS = {
  gateway: "providerTabs.gatewayLabel",
  bedrock: "providerTabs.bedrockLabel",
  vertex: "providerTabs.vertexLabel",
  foundry: "providerTabs.foundryLabel",
} as const;

const PROVIDER_SUBTITLE_KEYS = {
  gateway: "providerTabs.gatewaySubtitle",
  bedrock: "providerTabs.bedrockSubtitle",
  vertex: "providerTabs.vertexSubtitle",
  foundry: "providerTabs.foundrySubtitle",
} as const;

const MODEL_PLACEHOLDER_KEYS = {
  gateway: "form.modelPlaceholderGateway",
  bedrock: "form.modelPlaceholderBedrock",
  vertex: "form.modelPlaceholderVertex",
  foundry: "form.modelPlaceholderFoundry",
} as const;

const TEXTAREA_STYLE = {
  width: "100%",
  padding: "var(--sp-8) var(--sp-10)",
  background: "var(--bg-raised)",
  border: "var(--bw-hair) solid var(--line)",
  borderRadius: "var(--r-2)",
  color: "var(--fg)",
  fontFamily: "inherit",
  fontSize: "var(--fs-sm)",
  resize: "vertical",
} as const;

export function RouteForm({
  mode,
  initial,
  onSubmit,
  onCancel,
}: RouteFormProps) {
  const { t } = useTranslation("providers");
  const [providerKind, setProviderKind] = useState<RouteProviderKind>(
    initial?.provider_kind ?? "gateway",
  );

  // Common fields
  const [name, setName] = useState(initial?.name ?? "");
  const [model, setModel] = useState(initial?.model ?? "");
  const [smallFastModel, setSmallFastModel] = useState(
    initial?.small_fast_model ?? "",
  );
  const [additionalModels, setAdditionalModels] = useState(
    (initial?.additional_models ?? []).join("\n"),
  );
  const [wrapperOverride, setWrapperOverride] = useState(
    mode === "edit" ? initial?.wrapper_name ?? "" : "",
  );
  const [autoSlug, setAutoSlug] = useState("claude-route");

  // `use_keychain` is shared across provider variants — a route is
  // either keychain-backed or plaintext-backed, regardless of which
  // provider it talks to. Read from the details in edit mode.
  const initialUseKeychain = initial?.use_keychain ?? false;

  // Gateway state — hydrated from initial.gateway in edit mode.
  const [gwBase, setGwBase] = useState(initial?.gateway?.base_url ?? "");
  const [gwKey, setGwKey] = useState("");
  const [gwAuth, setGwAuth] = useState<"bearer" | "basic">(
    initial?.gateway?.auth_scheme === "basic" ? "basic" : "bearer",
  );
  const [gwToolSearch, setGwToolSearch] = useState(
    initial?.gateway?.enable_tool_search ?? false,
  );
  const [gwUseKeychain, setGwUseKeychain] = useState(initialUseKeychain);

  // Bedrock state — hydrated from initial.bedrock.
  const [bedRegion, setBedRegion] = useState(initial?.bedrock?.region ?? "");
  const [bedToken, setBedToken] = useState("");
  const [bedBaseUrl, setBedBaseUrl] = useState(
    initial?.bedrock?.base_url ?? "",
  );
  const [bedProfile, setBedProfile] = useState(
    initial?.bedrock?.aws_profile ?? "",
  );
  const [bedSkipAuth, setBedSkipAuth] = useState(
    initial?.bedrock?.skip_aws_auth ?? false,
  );
  const [bedUseKeychain, setBedUseKeychain] = useState(initialUseKeychain);

  // Vertex state (no inline secret — no keychain option).
  const [vxProjectId, setVxProjectId] = useState(
    initial?.vertex?.project_id ?? "",
  );
  const [vxRegion, setVxRegion] = useState(initial?.vertex?.region ?? "");
  const [vxBaseUrl, setVxBaseUrl] = useState(initial?.vertex?.base_url ?? "");
  const [vxSkipAuth, setVxSkipAuth] = useState(
    initial?.vertex?.skip_gcp_auth ?? false,
  );

  // Foundry state — hydrated from initial.foundry.
  const [fdKey, setFdKey] = useState("");
  const [fdBase, setFdBase] = useState(initial?.foundry?.base_url ?? "");
  const [fdResource, setFdResource] = useState(
    initial?.foundry?.resource ?? "",
  );
  const [fdSkipAuth, setFdSkipAuth] = useState(
    initial?.foundry?.skip_azure_auth ?? false,
  );
  const [fdUseKeychain, setFdUseKeychain] = useState(initialUseKeychain);

  const [submitting, setSubmitting] = useState(false);

  // Sticky hint: was this form opened via the network-detection
  // panel's "Use a provider" button? Drives preset emphasis.
  // The breadcrumb key is read here without clearing — ThirdPartySection
  // clears it when the modal closes. See `lib/networkPanelDeepLink.ts`.
  const [fromNetworkPanel] = useState(() => readFromNetworkPanelBreadcrumb());

  // Auto-derive slug preview from model field.
  useEffect(() => {
    let cancelled = false;
    if (!model.trim()) {
      setAutoSlug("claude-route");
      return;
    }
    void api
      .routesDeriveSlug(model.trim())
      .then((s) => {
        if (!cancelled) setAutoSlug(s);
      })
      .catch(() => {
        if (!cancelled) setAutoSlug("claude-route");
      });
    return () => {
      cancelled = true;
    };
  }, [model]);

  const wrapperPreview = wrapperOverride.trim() || autoSlug;
  const canSubmit = !submitting && name.trim() && model.trim() && providerReady();

  function providerReady(): boolean {
    if (providerKind === "gateway") {
      // In edit mode, allow blank api_key — Rust keeps the existing one.
      const keyOk = mode === "edit" || gwKey.length > 0;
      return gwBase.trim().length > 0 && keyOk;
    }
    if (providerKind === "bedrock") {
      return bedRegion.trim().length > 0;
    }
    if (providerKind === "vertex") {
      return vxProjectId.trim().length > 0;
    }
    if (providerKind === "foundry") {
      return fdBase.trim().length > 0 || fdResource.trim().length > 0;
    }
    return false;
  }

  const submit = async () => {
    if (!canSubmit) return;
    setSubmitting(true);
    const additional = additionalModels
      .split(/[\n,]/)
      .map((m) => m.trim())
      .filter(Boolean);

    const gateway: GatewayInputDto | null =
      providerKind === "gateway"
        ? {
            base_url: gwBase.trim(),
            api_key: gwKey,
            auth_scheme: gwAuth,
            enable_tool_search: gwToolSearch,
            use_keychain: gwUseKeychain,
          }
        : null;
    const bedrock: BedrockInputDto | null =
      providerKind === "bedrock"
        ? {
            region: bedRegion.trim(),
            bearer_token: bedToken,
            base_url: bedBaseUrl.trim(),
            aws_profile: bedProfile.trim(),
            skip_aws_auth: bedSkipAuth,
            use_keychain: bedUseKeychain,
          }
        : null;
    const vertex: VertexInputDto | null =
      providerKind === "vertex"
        ? {
            project_id: vxProjectId.trim(),
            region: vxRegion.trim(),
            base_url: vxBaseUrl.trim(),
            skip_gcp_auth: vxSkipAuth,
          }
        : null;
    const foundry: FoundryInputDto | null =
      providerKind === "foundry"
        ? {
            api_key: fdKey,
            base_url: fdBase.trim(),
            resource: fdResource.trim(),
            skip_azure_auth: fdSkipAuth,
            use_keychain: fdUseKeychain,
          }
        : null;

    const base = {
      name: name.trim(),
      provider_kind: providerKind,
      gateway,
      bedrock,
      vertex,
      foundry,
      model: model.trim(),
      small_fast_model: smallFastModel.trim() || null,
      additional_models: additional,
      wrapper_name: wrapperOverride.trim(),
    };

    const payload =
      mode === "edit" && initial
        ? ({ id: initial.id, ...base } as RouteUpdateDto)
        : (base as RouteCreateDto);

    try {
      await onSubmit(payload);
    } finally {
      // Clear local secret state on every code path (success, error,
      // user cancellation mid-submit). The earlier impl only cleared
      // on success, leaving the secret resident in React state until
      // the modal closed when a submit failed.
      setGwKey("");
      setBedToken("");
      setFdKey("");
      setSubmitting(false);
    }
  };

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-16)",
      }}
    >
      <ProviderTabs
        active={providerKind}
        onChange={setProviderKind}
        disabled={mode === "edit"}
      />

      <FieldBlock label={t("form.displayName")} htmlFor="route-name">
        <Input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder={t("form.displayNamePlaceholder")}
        />
      </FieldBlock>

      {providerKind === "gateway" && (
        <GatewayFields
          baseUrl={gwBase}
          setBaseUrl={setGwBase}
          apiKey={gwKey}
          setApiKey={setGwKey}
          authScheme={gwAuth}
          setAuthScheme={setGwAuth}
          enableToolSearch={gwToolSearch}
          setEnableToolSearch={setGwToolSearch}
          useKeychain={gwUseKeychain}
          setUseKeychain={setGwUseKeychain}
          editKeyHint={mode === "edit"}
          mode={mode}
          setModel={setModel}
          fromNetworkPanel={fromNetworkPanel}
        />
      )}
      {providerKind === "bedrock" && (
        <BedrockFields
          region={bedRegion}
          setRegion={setBedRegion}
          bearerToken={bedToken}
          setBearerToken={setBedToken}
          baseUrl={bedBaseUrl}
          setBaseUrl={setBedBaseUrl}
          awsProfile={bedProfile}
          setAwsProfile={setBedProfile}
          skipAuth={bedSkipAuth}
          setSkipAuth={setBedSkipAuth}
          useKeychain={bedUseKeychain}
          setUseKeychain={setBedUseKeychain}
          editKeyHint={mode === "edit"}
          mode={mode}
        />
      )}
      {providerKind === "vertex" && (
        <VertexFields
          projectId={vxProjectId}
          setProjectId={setVxProjectId}
          region={vxRegion}
          setRegion={setVxRegion}
          baseUrl={vxBaseUrl}
          setBaseUrl={setVxBaseUrl}
          skipAuth={vxSkipAuth}
          setSkipAuth={setVxSkipAuth}
        />
      )}
      {providerKind === "foundry" && (
        <FoundryFields
          apiKey={fdKey}
          setApiKey={setFdKey}
          baseUrl={fdBase}
          setBaseUrl={setFdBase}
          resource={fdResource}
          setResource={setFdResource}
          skipAuth={fdSkipAuth}
          setSkipAuth={setFdSkipAuth}
          useKeychain={fdUseKeychain}
          setUseKeychain={setFdUseKeychain}
          editKeyHint={mode === "edit"}
          mode={mode}
        />
      )}

      <FieldBlock label={t("form.defaultModel")} htmlFor="route-model">
        <Input
          value={model}
          onChange={(e) => setModel(e.target.value)}
          placeholder={t(MODEL_PLACEHOLDER_KEYS[providerKind])}
          glyph={NF.cpu}
        />
      </FieldBlock>

      <FieldBlock
        label={t("form.smallFastModel")}
        htmlFor="route-fast-model"
      >
        <Input
          value={smallFastModel}
          onChange={(e) => setSmallFastModel(e.target.value)}
          placeholder={t("form.smallFastModelPlaceholder")}
        />
      </FieldBlock>

      <FieldBlock
        label={t("form.additionalModels")}
        htmlFor="route-extras"
      >
        <textarea
          value={additionalModels}
          onChange={(e) => setAdditionalModels(e.target.value)}
          rows={2}
          placeholder="extra-model-id-1&#10;extra-model-id-2"
          style={TEXTAREA_STYLE}
        />
      </FieldBlock>

      <FieldBlock
        label={t("form.wrapperCommand", { slug: autoSlug })}
        htmlFor="route-wrapper"
      >
        <Input
          value={wrapperOverride}
          onChange={(e) => setWrapperOverride(e.target.value)}
          placeholder={autoSlug}
          glyph={NF.terminal}
        />
      </FieldBlock>

      <div
        style={{
          display: "flex",
          justifyContent: "flex-end",
          gap: "var(--sp-8)",
          marginTop: "var(--sp-8)",
        }}
      >
        <Button onClick={onCancel} variant="ghost" disabled={submitting}>
          {t("form.cancel")}
        </Button>
        <Button
          onClick={submit}
          variant="solid"
          disabled={!canSubmit}
          title={
            canSubmit
              ? mode === "edit"
                ? t("form.saveChangesTitle", { wrapper: wrapperPreview })
                : t("form.createRouteTitle", { wrapper: wrapperPreview })
              : t("form.fillRequired")
          }
        >
          {submitting
            ? mode === "edit"
              ? t("form.saving")
              : t("form.adding")
            : mode === "edit"
              ? t("form.save")
              : t("form.addRoute")}
        </Button>
      </div>
    </div>
  );
}

function ProviderTabs({
  active,
  onChange,
  disabled,
}: {
  active: RouteProviderKind;
  onChange: (k: RouteProviderKind) => void;
  disabled: boolean;
}) {
  const { t } = useTranslation("providers");
  return (
    <div role="tablist" aria-label={t("providerTabs.ariaLabel")}>
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(4, 1fr)",
          gap: "var(--sp-4)",
          padding: "var(--sp-4)",
          background: "var(--bg-sunken)",
          borderRadius: "var(--r-2)",
          border: "var(--bw-hair) solid var(--line)",
        }}
      >
        {PROVIDER_KINDS.map((kind) => (
          <button
            key={kind}
            type="button"
            role="tab"
            aria-selected={active === kind}
            disabled={disabled}
            onClick={() => onChange(kind)}
            title={t(PROVIDER_SUBTITLE_KEYS[kind])}
            style={{
              padding: "var(--sp-6) var(--sp-8)",
              border: "none",
              borderRadius: "var(--r-1)",
              background:
                active === kind ? "var(--bg-raised)" : "transparent",
              color:
                active === kind ? "var(--fg)" : "var(--fg-faint)",
              fontFamily: "inherit",
              fontSize: "var(--fs-sm)",
              fontWeight: active === kind ? 600 : 400,
              opacity: disabled && active !== kind ? 0.4 : 1,
            }}
          >
            {t(PROVIDER_LABEL_KEYS[kind])}
          </button>
        ))}
      </div>
      <p
        style={{
          margin: "var(--sp-6) 0 0",
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-faint)",
        }}
      >
        {t(PROVIDER_SUBTITLE_KEYS[active])}
        {disabled && t("providerTabs.lockedNote")}
      </p>
    </div>
  );
}

function SecretFieldHint({ editing }: { editing: boolean }) {
  const { t } = useTranslation("providers");
  if (!editing) return null;
  return (
    <p
      style={{
        margin: 0,
        fontSize: "var(--fs-2xs)",
        color: "var(--fg-faint)",
      }}
    >
      {t("form.secretHint")}
    </p>
  );
}

function KeychainOption(props: {
  checked: boolean;
  onChange: (b: boolean) => void;
  disabled: boolean;
  /** Which secret this checkbox governs, already localized by the
   *  caller — it is interpolated into `form.keychainLabel`. */
  field: string;
}) {
  const { t } = useTranslation("providers");
  const id = useId();
  const noteId = `${id}-note`;
  return (
    // `htmlFor` + `aria-describedby` rather than a wrapping `<label>`.
    // A wrapping label takes its accessible name from all of its text,
    // so this control announced as its label PLUS the whole `code`-laden
    // note after it. Name and description are different relationships.
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-8)",
        fontSize: "var(--fs-sm)",
        color: "var(--fg)",
        opacity: props.disabled ? 0.6 : 1,
      }}
      title={t("form.keychainTitle")}
    >
      <input
        id={id}
        type="checkbox"
        checked={props.checked}
        disabled={props.disabled}
        aria-describedby={noteId}
        onChange={(e) => props.onChange(e.target.checked)}
      />
      <label htmlFor={id}>{t("form.keychainLabel", { field: props.field })}</label>
      <span id={noteId} style={{ color: "var(--fg-faint)" }}>
        <Trans
          ns="providers"
          i18nKey="form.keychainNote"
          components={{ code: <code /> }}
        />
      </span>
    </div>
  );
}

/**
 * Curated gateway-provider presets. Each entry pre-fills base URL +
 * a sensible default model so users don't have to copy-paste from
 * vendor docs. The list deliberately leans toward providers reachable
 * from regions where Anthropic itself is blocked (mainland China),
 * since this is the catalog the network-detection panel routes users
 * to. See `dev-docs/network-detection-panel.md`.
 *
 * Endpoint paths are the OpenAI-compatible chat-completions roots —
 * the gateway wrapper handles the request-shape translation. If a
 * vendor changes their endpoint, update here; the form just plumbs
 * the string through.
 */
type GatewayPresetId =
  | "deepseek"
  | "moonshot"
  | "qwen"
  | "glm"
  | "openrouter"
  | "ollama";

interface GatewayPreset {
  id: GatewayPresetId;
  /**
   * Vendor brand name — data, never translated. `null` when the
   * visible label carries English copy alongside the brand ("Ollama
   * (local)"); that one is resolved from the catalog at render time.
   */
  label: string | null;
  baseUrl: string;
  model: string;
  /** True when reachable from networks that block Anthropic (China et
   *  al). Drives the "reachable here" hint when the form is opened
   *  from the network-detection panel. */
  reachableFromBlockedRegions: boolean;
}

/**
 * Endpoints and model ids stay here — they are vendor data. The
 * per-preset `note` is prose and moved to `presets.*Note`, resolved
 * per render so a language switch reaches an already-open form.
 */
const GATEWAY_PRESETS: GatewayPreset[] = [
  {
    id: "deepseek",
    label: "DeepSeek",
    baseUrl: "https://api.deepseek.com/v1",
    model: "deepseek-chat",
    reachableFromBlockedRegions: true,
  },
  {
    id: "moonshot",
    label: "Kimi (Moonshot)",
    baseUrl: "https://api.moonshot.cn/v1",
    model: "moonshot-v1-32k",
    reachableFromBlockedRegions: true,
  },
  {
    id: "qwen",
    label: "Qwen (DashScope)",
    baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    model: "qwen-coder-plus",
    reachableFromBlockedRegions: true,
  },
  {
    id: "glm",
    label: "GLM (Zhipu)",
    baseUrl: "https://open.bigmodel.cn/api/paas/v4",
    model: "glm-4-plus",
    reachableFromBlockedRegions: true,
  },
  {
    id: "openrouter",
    label: "OpenRouter",
    baseUrl: "https://openrouter.ai/api/v1",
    model: "moonshotai/kimi-k2",
    reachableFromBlockedRegions: false,
  },
  {
    id: "ollama",
    label: null,
    baseUrl: "http://127.0.0.1:11434/v1",
    model: "llama3.2:3b",
    reachableFromBlockedRegions: true,
  },
];

function GatewayPresetsBar({
  setBaseUrl,
  setModel,
  highlight,
}: {
  setBaseUrl: (s: string) => void;
  setModel: (s: string) => void;
  /** When true, emphasize the China-reachable presets — the form was
   *  opened from the network-detection panel. */
  highlight: boolean;
}) {
  const { t } = useTranslation("providers");
  // Built per render, not at module scope, so a language switch
  // repaints an already-open form.
  const notes: Record<GatewayPresetId, string> = {
    deepseek: t("presets.deepseekNote"),
    moonshot: t("presets.moonshotNote"),
    qwen: t("presets.qwenNote"),
    glm: t("presets.glmNote"),
    openrouter: t("presets.openrouterNote"),
    ollama: t("presets.ollamaNote"),
  };
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-6)",
        padding: "var(--sp-10) var(--sp-12)",
        background: highlight
          ? "color-mix(in oklch, var(--accent) 8%, var(--bg-raised))"
          : "var(--bg-sunken)",
        border: highlight
          ? "var(--bw-hair) solid var(--accent)"
          : "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
      }}
    >
      <div
        style={{
          fontSize: "var(--fs-xs)",
          color: "var(--fg-muted)",
        }}
      >
        {highlight
          ? t("presets.quickStartReachable")
          : t("presets.quickStartKnown")}
      </div>
      <div
        style={{
          display: "flex",
          flexWrap: "wrap",
          gap: "var(--sp-6)",
        }}
      >
        {GATEWAY_PRESETS.filter(
          (p) => !highlight || p.reachableFromBlockedRegions,
        ).map((p) => (
          <button
            key={p.id}
            type="button"
            onClick={() => {
              setBaseUrl(p.baseUrl);
              setModel(p.model);
            }}
            title={notes[p.id]}
            style={{
              padding: "var(--sp-4) var(--sp-10)",
              fontSize: "var(--fs-xs)",
              fontFamily: "inherit",
              color: "var(--fg)",
              background: "var(--bg-raised)",
              border: "var(--bw-hair) solid var(--line)",
              borderRadius: "var(--r-1)",
            }}
          >
            {p.label ?? t("presets.ollamaLabel")}
          </button>
        ))}
      </div>
    </div>
  );
}

function GatewayFields(props: {
  baseUrl: string;
  setBaseUrl: (s: string) => void;
  apiKey: string;
  setApiKey: (s: string) => void;
  authScheme: "bearer" | "basic";
  setAuthScheme: (s: "bearer" | "basic") => void;
  enableToolSearch: boolean;
  setEnableToolSearch: (b: boolean) => void;
  useKeychain: boolean;
  setUseKeychain: (b: boolean) => void;
  editKeyHint: boolean;
  mode: "add" | "edit";
  /** Set the route's default model. Plumbed through so the preset
   *  buttons can pre-fill it alongside base URL. */
  setModel?: (s: string) => void;
  /** True when the form was opened via the network-detection panel.
   *  Highlights the China-reachable preset subset. */
  fromNetworkPanel?: boolean;
}) {
  const { t } = useTranslation("providers");
  return (
    <>
      {props.mode === "add" && props.setModel && (
        <GatewayPresetsBar
          setBaseUrl={props.setBaseUrl}
          setModel={props.setModel}
          highlight={props.fromNetworkPanel ?? false}
        />
      )}
      <FieldBlock label={t("form.baseUrl")} htmlFor="route-base">
        <Input
          value={props.baseUrl}
          onChange={(e) => props.setBaseUrl(e.target.value)}
          placeholder="http://127.0.0.1:11434/v1"
          glyph={NF.globe}
        />
      </FieldBlock>
      <FieldBlock label={t("form.apiKey")} htmlFor="route-key">
        <Input
          value={props.apiKey}
          onChange={(e) => props.setApiKey(e.target.value)}
          placeholder={
            props.editKeyHint
              ? t("form.unchangedPlaceholder")
              : t("form.gatewayKeyPlaceholder")
          }
          type="password"
          glyph={NF.key}
        />
      </FieldBlock>
      <SecretFieldHint editing={props.editKeyHint} />

      <FieldBlock label={t("form.authScheme")} htmlFor="route-auth">
        <select
          value={props.authScheme}
          onChange={(e) =>
            props.setAuthScheme(e.target.value as "bearer" | "basic")
          }
          style={{
            height: "var(--input-height)",
            padding: "0 var(--sp-10)",
            background: "var(--bg-raised)",
            border: "var(--bw-hair) solid var(--line)",
            borderRadius: "var(--r-2)",
            color: "var(--fg)",
            fontFamily: "inherit",
            fontSize: "var(--fs-sm)",
          }}
        >
          <option value="bearer">{t("form.authBearer")}</option>
          <option value="basic">{t("form.authBasic")}</option>
        </select>
      </FieldBlock>

      <label
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-8)",
          fontSize: "var(--fs-sm)",
          color: "var(--fg)",
        }}
      >
        <input
          type="checkbox"
          checked={props.enableToolSearch}
          onChange={(e) => props.setEnableToolSearch(e.target.checked)}
        />
        <Trans
          ns="providers"
          i18nKey="form.toolSearchLabel"
          components={{ code: <code /> }}
        />
        <span style={{ color: "var(--fg-faint)" }}>
          {t("form.toolSearchNote")}
        </span>
      </label>
      <KeychainOption
        checked={props.useKeychain}
        onChange={props.setUseKeychain}
        disabled={props.mode === "edit"}
        field={t("form.fieldApiKey")}
      />
    </>
  );
}

function BedrockFields(props: {
  region: string;
  setRegion: (s: string) => void;
  bearerToken: string;
  setBearerToken: (s: string) => void;
  baseUrl: string;
  setBaseUrl: (s: string) => void;
  awsProfile: string;
  setAwsProfile: (s: string) => void;
  skipAuth: boolean;
  setSkipAuth: (b: boolean) => void;
  useKeychain: boolean;
  setUseKeychain: (b: boolean) => void;
  editKeyHint: boolean;
  mode: "add" | "edit";
}) {
  const { t } = useTranslation("providers");
  return (
    <>
      <FieldBlock label={t("form.awsRegion")} htmlFor="route-bed-region">
        <Input
          value={props.region}
          onChange={(e) => props.setRegion(e.target.value)}
          placeholder="us-west-2"
        />
      </FieldBlock>
      <FieldBlock
        label={t("form.bedrockToken")}
        htmlFor="route-bed-token"
      >
        <Input
          value={props.bearerToken}
          onChange={(e) => props.setBearerToken(e.target.value)}
          placeholder={
            props.editKeyHint
              ? t("form.unchangedPlaceholder")
              : t("form.bedrockTokenPlaceholder")
          }
          type="password"
          glyph={NF.key}
        />
      </FieldBlock>
      <SecretFieldHint editing={props.editKeyHint} />

      <FieldBlock
        label={t("form.awsProfile")}
        htmlFor="route-bed-profile"
      >
        <Input
          value={props.awsProfile}
          onChange={(e) => props.setAwsProfile(e.target.value)}
          placeholder={t("form.awsProfilePlaceholder")}
        />
      </FieldBlock>
      <FieldBlock
        label={t("form.baseUrlOverride")}
        htmlFor="route-bed-base"
      >
        <Input
          value={props.baseUrl}
          onChange={(e) => props.setBaseUrl(e.target.value)}
          placeholder={t("form.bedrockBaseUrlPlaceholder")}
          glyph={NF.globe}
        />
      </FieldBlock>
      <label
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-8)",
          fontSize: "var(--fs-sm)",
          color: "var(--fg)",
        }}
      >
        <input
          type="checkbox"
          checked={props.skipAuth}
          onChange={(e) => props.setSkipAuth(e.target.checked)}
        />
        {t("form.skipBedrockAuth")}
        <span style={{ color: "var(--fg-faint)" }}>
          {t("form.skipBedrockAuthNote")}
        </span>
      </label>
      <KeychainOption
        checked={props.useKeychain}
        onChange={props.setUseKeychain}
        disabled={props.mode === "edit"}
        field={t("form.fieldBearerToken")}
      />
    </>
  );
}

function VertexFields(props: {
  projectId: string;
  setProjectId: (s: string) => void;
  region: string;
  setRegion: (s: string) => void;
  baseUrl: string;
  setBaseUrl: (s: string) => void;
  skipAuth: boolean;
  setSkipAuth: (b: boolean) => void;
}) {
  const { t } = useTranslation("providers");
  return (
    <>
      <FieldBlock label={t("form.gcpProjectId")} htmlFor="route-vx-project">
        <Input
          value={props.projectId}
          onChange={(e) => props.setProjectId(e.target.value)}
          placeholder="my-gcp-project"
        />
      </FieldBlock>
      <FieldBlock label={t("form.regionOptional")} htmlFor="route-vx-region">
        <Input
          value={props.region}
          onChange={(e) => props.setRegion(e.target.value)}
          placeholder={t("form.vertexRegionPlaceholder")}
        />
      </FieldBlock>
      <FieldBlock label={t("form.baseUrlOverride")} htmlFor="route-vx-base">
        <Input
          value={props.baseUrl}
          onChange={(e) => props.setBaseUrl(e.target.value)}
          placeholder={t("form.vertexBaseUrlPlaceholder")}
          glyph={NF.globe}
        />
      </FieldBlock>
      <label
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-8)",
          fontSize: "var(--fs-sm)",
          color: "var(--fg)",
        }}
      >
        <input
          type="checkbox"
          checked={props.skipAuth}
          onChange={(e) => props.setSkipAuth(e.target.checked)}
        />
        {t("form.skipVertexAuth")}
        <span style={{ color: "var(--fg-faint)" }}>
          {t("form.skipVertexAuthNote")}
        </span>
      </label>
    </>
  );
}

function FoundryFields(props: {
  apiKey: string;
  setApiKey: (s: string) => void;
  baseUrl: string;
  setBaseUrl: (s: string) => void;
  resource: string;
  setResource: (s: string) => void;
  skipAuth: boolean;
  setSkipAuth: (b: boolean) => void;
  useKeychain: boolean;
  setUseKeychain: (b: boolean) => void;
  editKeyHint: boolean;
  mode: "add" | "edit";
}) {
  const { t } = useTranslation("providers");
  return (
    <>
      <p
        style={{
          margin: 0,
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-faint)",
        }}
      >
        {t("form.foundryEither")}
      </p>
      <FieldBlock label={t("form.baseUrl")} htmlFor="route-fd-base">
        <Input
          value={props.baseUrl}
          onChange={(e) => props.setBaseUrl(e.target.value)}
          placeholder="https://my-resource.openai.azure.com"
          glyph={NF.globe}
          disabled={props.resource.length > 0}
        />
      </FieldBlock>
      <FieldBlock label={t("form.resourceName")} htmlFor="route-fd-resource">
        <Input
          value={props.resource}
          onChange={(e) => props.setResource(e.target.value)}
          placeholder={t("form.resourcePlaceholder")}
          disabled={props.baseUrl.length > 0}
        />
      </FieldBlock>
      <FieldBlock label={t("form.foundryApiKey")} htmlFor="route-fd-key">
        <Input
          value={props.apiKey}
          onChange={(e) => props.setApiKey(e.target.value)}
          placeholder={
            props.editKeyHint
              ? t("form.unchangedPlaceholder")
              : t("form.foundryKeyPlaceholder")
          }
          type="password"
          glyph={NF.key}
        />
      </FieldBlock>
      <SecretFieldHint editing={props.editKeyHint} />
      <label
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-8)",
          fontSize: "var(--fs-sm)",
          color: "var(--fg)",
        }}
      >
        <input
          type="checkbox"
          checked={props.skipAuth}
          onChange={(e) => props.setSkipAuth(e.target.checked)}
        />
        {t("form.skipFoundryAuth")}
      </label>
      <KeychainOption
        checked={props.useKeychain}
        onChange={props.setUseKeychain}
        disabled={props.mode === "edit"}
        field={t("form.fieldApiKey")}
      />
    </>
  );
}
