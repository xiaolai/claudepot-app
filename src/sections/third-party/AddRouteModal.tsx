import { useEffect, useState } from "react";
import { Modal } from "../../components/primitives/Modal";
import { Button } from "../../components/primitives/Button";
import { Input } from "../../components/primitives/Input";
import { FieldBlock } from "../../components/primitives/modalParts";
import { NF } from "../../icons";
import { api } from "../../api";
import type {
  RouteCreateDto,
  RouteSummaryDto,
} from "../../types";

interface AddRouteModalProps {
  open: boolean;
  onClose: () => void;
  onCreated: (route: RouteSummaryDto) => void;
  onError: (msg: string) => void;
}

const PLACEHOLDER_NAME = "e.g. Local Ollama";
const PLACEHOLDER_BASE = "http://127.0.0.1:11434";
const PLACEHOLDER_MODEL = "llama3.2:3b";
const PLACEHOLDER_KEY = "ollama (any string for local servers)";

/**
 * Phase 2 — gateway-only add-route form.
 * Bedrock/Vertex/Foundry land in phase 4.
 */
export function AddRouteModal({
  open,
  onClose,
  onCreated,
  onError,
}: AddRouteModalProps) {
  const [name, setName] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [model, setModel] = useState("");
  const [smallFastModel, setSmallFastModel] = useState("");
  const [additionalModels, setAdditionalModels] = useState("");
  const [wrapperOverride, setWrapperOverride] = useState("");
  const [autoSlug, setAutoSlug] = useState("claude-route");
  const [enableToolSearch, setEnableToolSearch] = useState(false);
  const [submitting, setSubmitting] = useState(false);

  // Keep the auto-slug preview in sync with the model field. Falls
  // back to "claude-route" on empty / errored derivation.
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

  // Reset form on close.
  useEffect(() => {
    if (!open) {
      setName("");
      setBaseUrl("");
      setApiKey("");
      setModel("");
      setSmallFastModel("");
      setAdditionalModels("");
      setWrapperOverride("");
      setEnableToolSearch(false);
      setSubmitting(false);
    }
  }, [open]);

  const wrapperPreview = wrapperOverride.trim() || autoSlug;
  const canSubmit =
    !submitting &&
    name.trim().length > 0 &&
    baseUrl.trim().length > 0 &&
    apiKey.length > 0 &&
    model.trim().length > 0;

  const submit = async () => {
    if (!canSubmit) return;
    setSubmitting(true);
    const payload: RouteCreateDto = {
      name: name.trim(),
      provider_kind: "gateway",
      gateway: {
        base_url: baseUrl.trim(),
        api_key: apiKey,
        auth_scheme: "bearer",
        enable_tool_search: enableToolSearch,
      },
      model: model.trim(),
      small_fast_model: smallFastModel.trim() || null,
      additional_models: additionalModels
        .split(/[\n,]/)
        .map((m) => m.trim())
        .filter(Boolean),
      wrapper_name: wrapperOverride.trim(),
    };
    try {
      const created = await api.routesAdd(payload);
      // Best-effort secret cleanup. The IPC arg-buffer is what we
      // actually want gone; calling routesZeroSecret is symbolic
      // (the API_KEY argument was already consumed Rust-side) but
      // also clears the renderer's local copy via setApiKey("").
      setApiKey("");
      onCreated(created);
      onClose();
    } catch (e) {
      onError(`Add route failed: ${e instanceof Error ? e.message : e}`);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Modal open={open} onClose={onClose} width="lg" aria-labelledby="add-route-title">
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "var(--sp-16)",
          padding: "var(--sp-20) var(--sp-24)",
        }}
      >
        <h2
          id="add-route-title"
          style={{
            margin: 0,
            fontSize: "var(--fs-lg)",
            fontWeight: 600,
            color: "var(--fg-strong)",
          }}
        >
          Add a third-party route
        </h2>
        <p
          style={{
            margin: 0,
            fontSize: "var(--fs-sm)",
            color: "var(--fg-faint)",
          }}
        >
          Configure a non-Anthropic LLM gateway. Bedrock, Vertex, and
          Foundry providers land in a later phase — gateway covers
          Ollama, vLLM, OpenRouter, Kimi, DeepSeek, GLM, LiteLLM, and
          any Anthropic-Messages-compatible endpoint.
        </p>

        <FieldBlock label="Display name" htmlFor="route-name">
          <Input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={PLACEHOLDER_NAME}
          />
        </FieldBlock>

        <FieldBlock label="Base URL" htmlFor="route-base">
          <Input
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            placeholder={PLACEHOLDER_BASE}
            glyph={NF.globe}
          />
        </FieldBlock>

        <FieldBlock label="API key" htmlFor="route-key">
          <Input
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            placeholder={PLACEHOLDER_KEY}
            type="password"
            glyph={NF.key}
          />
        </FieldBlock>

        <FieldBlock label="Default model" htmlFor="route-model">
          <Input
            value={model}
            onChange={(e) => setModel(e.target.value)}
            placeholder={PLACEHOLDER_MODEL}
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
            rows={3}
            placeholder="e.g.&#10;qwen2.5-coder:7b&#10;phi3:14b"
            style={{
              width: "100%",
              padding: "var(--sp-8) var(--sp-10)",
              background: "var(--bg-raised)",
              border: "var(--bw-hair) solid var(--line)",
              borderRadius: "var(--r-2)",
              color: "var(--fg)",
              fontFamily: "inherit",
              fontSize: "var(--fs-sm)",
              resize: "vertical",
            }}
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
            checked={enableToolSearch}
            onChange={(e) => setEnableToolSearch(e.target.checked)}
          />
          Enable <code>tool_reference</code> beta blocks
          <span style={{ color: "var(--fg-faint)" }}>
            — only if your gateway forwards Anthropic beta headers
          </span>
        </label>

        <div
          style={{
            display: "flex",
            justifyContent: "flex-end",
            gap: "var(--sp-8)",
            marginTop: "var(--sp-8)",
          }}
        >
          <Button onClick={onClose} variant="ghost" disabled={submitting}>
            Cancel
          </Button>
          <Button
            onClick={submit}
            variant="solid"
            disabled={!canSubmit}
            title={
              canSubmit
                ? `Create route — wrapper will be ${wrapperPreview}`
                : "Fill in name, base URL, API key, and model"
            }
          >
            {submitting ? "Adding…" : "Add route"}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
