import { useEffect, useState } from "react";
import { Button } from "../../components/primitives/Button";
import { Input } from "../../components/primitives/Input";
import { FieldBlock } from "../../components/primitives/modalParts";
import { NF } from "../../icons";
import { api } from "../../api";
import type {
  BedrockInputDto,
  FoundryInputDto,
  GatewayInputDto,
  RouteCreateDto,
  RouteProviderKind,
  RouteSummaryDto,
  RouteUpdateDto,
  VertexInputDto,
} from "../../types";

export interface RouteFormProps {
  mode: "add" | "edit";
  /**
   * Pre-population for edit mode. For security the api_key /
   * bearer_token / foundry_api_key are NOT returned from the
   * backend — those fields stay empty in the form and the
   * commit policy is "blank = keep existing on the Rust side."
   */
  initial?: RouteSummaryDto | null;
  onSubmit: (
    payload: RouteCreateDto | RouteUpdateDto,
  ) => Promise<void>;
  onCancel: () => void;
}

const PROVIDERS: { kind: RouteProviderKind; label: string; subtitle: string }[] =
  [
    {
      kind: "gateway",
      label: "Gateway",
      subtitle: "Ollama / OpenRouter / Kimi / vLLM / LiteLLM / any",
    },
    {
      kind: "bedrock",
      label: "Bedrock",
      subtitle: "Amazon Bedrock — region + IAM or bearer token",
    },
    {
      kind: "vertex",
      label: "Vertex",
      subtitle: "Google Cloud Vertex AI — project + region",
    },
    {
      kind: "foundry",
      label: "Foundry",
      subtitle: "Microsoft Azure AI Foundry — base URL or resource",
    },
  ];

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

  // Gateway state
  const [gwBase, setGwBase] = useState(
    initial?.provider_kind === "gateway" ? initial.base_url : "",
  );
  const [gwKey, setGwKey] = useState("");
  const [gwAuth, setGwAuth] = useState<"bearer" | "basic">(
    initial?.auth_scheme === "basic" ? "basic" : "bearer",
  );
  const [gwToolSearch, setGwToolSearch] = useState(
    initial?.enable_tool_search ?? false,
  );

  // Bedrock state
  const [bedRegion, setBedRegion] = useState("");
  const [bedToken, setBedToken] = useState("");
  const [bedBaseUrl, setBedBaseUrl] = useState("");
  const [bedProfile, setBedProfile] = useState("");
  const [bedSkipAuth, setBedSkipAuth] = useState(false);

  // Vertex state
  const [vxProjectId, setVxProjectId] = useState("");
  const [vxRegion, setVxRegion] = useState("");
  const [vxBaseUrl, setVxBaseUrl] = useState("");
  const [vxSkipAuth, setVxSkipAuth] = useState(false);

  // Foundry state
  const [fdKey, setFdKey] = useState("");
  const [fdBase, setFdBase] = useState("");
  const [fdResource, setFdResource] = useState("");
  const [fdSkipAuth, setFdSkipAuth] = useState(false);

  const [submitting, setSubmitting] = useState(false);

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
      // Clear local secret state best-effort.
      setGwKey("");
      setBedToken("");
      setFdKey("");
    } finally {
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

      <FieldBlock label="Display name" htmlFor="route-name">
        <Input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="e.g. Local Ollama, Bedrock prod, Kimi K2"
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
          editKeyHint={mode === "edit"}
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
          editKeyHint={mode === "edit"}
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
          editKeyHint={mode === "edit"}
        />
      )}

      <FieldBlock label="Default model" htmlFor="route-model">
        <Input
          value={model}
          onChange={(e) => setModel(e.target.value)}
          placeholder={modelPlaceholder(providerKind)}
          glyph={NF.cpu}
        />
      </FieldBlock>

      <FieldBlock
        label="Small/fast model (optional)"
        htmlFor="route-fast-model"
      >
        <Input
          value={smallFastModel}
          onChange={(e) => setSmallFastModel(e.target.value)}
          placeholder="defaults to default model"
        />
      </FieldBlock>

      <FieldBlock
        label="Additional models (optional, one per line)"
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
        label={`Wrapper command (defaults to ${autoSlug})`}
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
          Cancel
        </Button>
        <Button
          onClick={submit}
          variant="solid"
          disabled={!canSubmit}
          title={
            canSubmit
              ? `${mode === "edit" ? "Save changes" : "Create route"} — wrapper will be ${wrapperPreview}`
              : "Fill in the required fields"
          }
        >
          {submitting
            ? mode === "edit"
              ? "Saving…"
              : "Adding…"
            : mode === "edit"
              ? "Save"
              : "Add route"}
        </Button>
      </div>
    </div>
  );
}

function modelPlaceholder(kind: RouteProviderKind): string {
  switch (kind) {
    case "gateway":
      return "e.g. llama3.2:3b, moonshotai/kimi-k2";
    case "bedrock":
      return "e.g. us.anthropic.claude-sonnet-4-5-v1:0";
    case "vertex":
      return "e.g. claude-sonnet-4-5@20250929";
    case "foundry":
      return "e.g. claude-sonnet-4-5";
  }
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
  return (
    <div role="tablist" aria-label="Provider type">
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
        {PROVIDERS.map((p) => (
          <button
            key={p.kind}
            type="button"
            role="tab"
            aria-selected={active === p.kind}
            disabled={disabled}
            onClick={() => onChange(p.kind)}
            title={p.subtitle}
            style={{
              padding: "var(--sp-6) var(--sp-8)",
              border: "none",
              borderRadius: "var(--r-1)",
              background:
                active === p.kind ? "var(--bg-raised)" : "transparent",
              color:
                active === p.kind ? "var(--fg-strong)" : "var(--fg-faint)",
              fontFamily: "inherit",
              fontSize: "var(--fs-sm)",
              fontWeight: active === p.kind ? 600 : 400,
              cursor: disabled ? "not-allowed" : "pointer",
              opacity: disabled && active !== p.kind ? 0.4 : 1,
            }}
          >
            {p.label}
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
        {PROVIDERS.find((p) => p.kind === active)?.subtitle}
        {disabled && " — provider can't be changed when editing; delete and recreate to switch."}
      </p>
    </div>
  );
}

function SecretFieldHint({ editing }: { editing: boolean }) {
  if (!editing) return null;
  return (
    <p
      style={{
        margin: 0,
        fontSize: "var(--fs-2xs)",
        color: "var(--fg-faint)",
      }}
    >
      Leave blank to keep the existing secret unchanged.
    </p>
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
  editKeyHint: boolean;
}) {
  return (
    <>
      <FieldBlock label="Base URL" htmlFor="route-base">
        <Input
          value={props.baseUrl}
          onChange={(e) => props.setBaseUrl(e.target.value)}
          placeholder="http://127.0.0.1:11434"
          glyph={NF.globe}
        />
      </FieldBlock>
      <FieldBlock label="API key" htmlFor="route-key">
        <Input
          value={props.apiKey}
          onChange={(e) => props.setApiKey(e.target.value)}
          placeholder={
            props.editKeyHint
              ? "(unchanged) — type to replace"
              : "ollama (any string for local servers)"
          }
          type="password"
          glyph={NF.key}
        />
      </FieldBlock>
      <SecretFieldHint editing={props.editKeyHint} />

      <FieldBlock label="Auth scheme" htmlFor="route-auth">
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
          <option value="bearer">Bearer (default)</option>
          <option value="basic">Basic</option>
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
        Enable <code>tool_reference</code> beta blocks
        <span style={{ color: "var(--fg-faint)" }}>
          — only if your gateway forwards Anthropic beta headers
        </span>
      </label>
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
  editKeyHint: boolean;
}) {
  return (
    <>
      <FieldBlock label="AWS region" htmlFor="route-bed-region">
        <Input
          value={props.region}
          onChange={(e) => props.setRegion(e.target.value)}
          placeholder="us-west-2"
        />
      </FieldBlock>
      <FieldBlock
        label="Bedrock bearer token (optional)"
        htmlFor="route-bed-token"
      >
        <Input
          value={props.bearerToken}
          onChange={(e) => props.setBearerToken(e.target.value)}
          placeholder={
            props.editKeyHint
              ? "(unchanged) — type to replace"
              : "leave blank to use AWS profile"
          }
          type="password"
          glyph={NF.key}
        />
      </FieldBlock>
      <SecretFieldHint editing={props.editKeyHint} />

      <FieldBlock
        label="AWS profile name (optional)"
        htmlFor="route-bed-profile"
      >
        <Input
          value={props.awsProfile}
          onChange={(e) => props.setAwsProfile(e.target.value)}
          placeholder="e.g. claudepot-prod (from ~/.aws/credentials)"
        />
      </FieldBlock>
      <FieldBlock label="Base URL override (optional)" htmlFor="route-bed-base">
        <Input
          value={props.baseUrl}
          onChange={(e) => props.setBaseUrl(e.target.value)}
          placeholder="for LiteLLM-fronted Bedrock routes"
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
        Skip Bedrock auth
        <span style={{ color: "var(--fg-faint)" }}>
          — gateway handles AWS auth on the proxy side
        </span>
      </label>
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
  return (
    <>
      <FieldBlock label="GCP project ID" htmlFor="route-vx-project">
        <Input
          value={props.projectId}
          onChange={(e) => props.setProjectId(e.target.value)}
          placeholder="my-gcp-project"
        />
      </FieldBlock>
      <FieldBlock label="Region (optional)" htmlFor="route-vx-region">
        <Input
          value={props.region}
          onChange={(e) => props.setRegion(e.target.value)}
          placeholder="us-east5 (default)"
        />
      </FieldBlock>
      <FieldBlock label="Base URL override (optional)" htmlFor="route-vx-base">
        <Input
          value={props.baseUrl}
          onChange={(e) => props.setBaseUrl(e.target.value)}
          placeholder="for LiteLLM-fronted Vertex routes"
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
        Skip Vertex auth
        <span style={{ color: "var(--fg-faint)" }}>
          — gateway handles GCP auth
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
  editKeyHint: boolean;
}) {
  return (
    <>
      <p
        style={{
          margin: 0,
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-faint)",
        }}
      >
        Set EITHER a base URL OR a resource name — not both.
      </p>
      <FieldBlock label="Base URL" htmlFor="route-fd-base">
        <Input
          value={props.baseUrl}
          onChange={(e) => props.setBaseUrl(e.target.value)}
          placeholder="https://my-resource.openai.azure.com"
          glyph={NF.globe}
          disabled={props.resource.length > 0}
        />
      </FieldBlock>
      <FieldBlock label="Resource name" htmlFor="route-fd-resource">
        <Input
          value={props.resource}
          onChange={(e) => props.setResource(e.target.value)}
          placeholder="my-resource (Azure resource name)"
          disabled={props.baseUrl.length > 0}
        />
      </FieldBlock>
      <FieldBlock label="Foundry API key" htmlFor="route-fd-key">
        <Input
          value={props.apiKey}
          onChange={(e) => props.setApiKey(e.target.value)}
          placeholder={
            props.editKeyHint
              ? "(unchanged) — type to replace"
              : "Azure Foundry API key"
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
        Skip Foundry auth
      </label>
    </>
  );
}
