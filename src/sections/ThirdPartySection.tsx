import { ScreenHeader } from "../shell/ScreenHeader";

/**
 * Third-party section — entry point for non-Anthropic LLM routes.
 *
 * Phase 0 stub. Full design in `dev-docs/third-party-llm-design.md`.
 *
 * Forthcoming:
 *  - CLI wrappers under `~/.claudepot/bin/<name>` that set
 *    `ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN` and exec `claude`.
 *  - Desktop profiles under
 *    `~/Library/Application Support/Claude-3p/configLibrary/<uuid>.json`,
 *    coexisting with the first-party Anthropic identity.
 *  - Provider support: gateway (Ollama / OpenRouter / Kimi / DeepSeek
 *    / GLM / LiteLLM), Bedrock, Vertex, Foundry.
 */
export function ThirdPartySection() {
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        overflow: "hidden",
      }}
    >
      <ScreenHeader
        title="Third-party"
        subtitle="Run Claude Code and Claude Desktop with non-Anthropic LLMs"
      />
      <div
        style={{
          flex: 1,
          overflow: "auto",
          padding: "var(--sp-32)",
          maxWidth: 720,
          color: "var(--fg)",
          fontSize: "var(--fs-sm)",
          lineHeight: 1.6,
        }}
      >
        <p style={{ marginTop: 0 }}>
          Manage routes to non-Anthropic backends — Bedrock, Vertex,
          Foundry, or any Anthropic-Messages-compatible gateway
          (Ollama, vLLM, OpenRouter, Kimi, DeepSeek, GLM, LiteLLM,
          and more).
        </p>
        <p>
          Each route materializes as a wrapper command on PATH —{" "}
          <code style={{ color: "var(--fg-strong)" }}>claude-llama3</code>
          ,{" "}
          <code style={{ color: "var(--fg-strong)" }}>claude-kimi</code>
          ,{" "}
          <code style={{ color: "var(--fg-strong)" }}>
            claude-bedrock-prod
          </code>{" "}
          — and as an entry in Claude Desktop&rsquo;s native
          configuration registry. The first-party{" "}
          <code style={{ color: "var(--fg-strong)" }}>claude</code>{" "}
          binary and your Anthropic account are never touched.
        </p>
        <p style={{ color: "var(--fg-faint)", fontStyle: "italic" }}>
          Profile management UI is in progress.
        </p>
      </div>
    </div>
  );
}
