import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Network requirements & troubleshooting",
  description:
    "What endpoints Claudepot needs, how to diagnose unreachability, and what to do when Anthropic's API isn't reachable from your network.",
};

export default function NetworkHelpPage() {
  return (
    <div className="proto-page-aside">
      <nav
        className="proto-page-aside-nav proto-page-aside-nav--mobile-hide"
        aria-label="On this page"
      >
        <span className="proto-page-aside-nav-title">On this page</span>
        <ul>
          <li><a href="#what-claudepot-needs">What Claudepot needs</a></li>
          <li><a href="#diagnose">Diagnose the failure</a></li>
          <li><a href="#proxy">Configure a proxy</a></li>
          <li><a href="#install-mirrors">Faster CLI install (mirrors)</a></li>
          <li><a href="#third-party">Use a third-party LLM</a></li>
          <li><a href="#out-of-scope">What we don't cover</a></li>
        </ul>
      </nav>

      <div className="proto-page-aside-content">
        <h1>Network requirements & troubleshooting</h1>
        <p className="proto-dek">
          When Claudepot can't reach Anthropic's API, the in-app panel
          links here. This page explains what endpoints the app and CLI
          need, how to tell <em>which</em> network path is broken, and
          which remediation fits which failure.
        </p>

        <section id="what-claudepot-needs" className="proto-section">
          <h2>What Claudepot needs</h2>
          <p>
            Claudepot itself uses three Anthropic-hosted endpoints:
          </p>
          <ul>
            <li>
              <code>api.anthropic.com</code> &mdash; the actual API. All
              account verification, usage data, and credential refreshes
              go through this host. <strong>This is the one whose
              unreachability triggers the in-app panel.</strong>
            </li>
            <li>
              <code>platform.claude.com</code> &mdash; the OAuth token
              endpoint used during account login.
            </li>
            <li>
              <code>claude.ai</code> &mdash; the OAuth login surface
              (browser-based). Only relevant during initial sign-in for
              first-party Claude accounts.
            </li>
          </ul>
          <p>
            Claudepot also fetches <code>status.claude.com</code> for
            service-status indicators, but that's optional and degrades
            gracefully when blocked.
          </p>
        </section>

        <section id="diagnose" className="proto-section">
          <h2>Diagnose the failure</h2>
          <p>
            "I can't reach Anthropic" has at least four distinct shapes,
            and the right fix depends on which one. Run this in a
            terminal:
          </p>
          <pre>
            <code>curl -v https://api.anthropic.com/v1/health</code>
          </pre>
          <p>The interesting bit is what fails:</p>
          <ul>
            <li>
              <strong>DNS failure</strong> ("Could not resolve host",
              "Name or service not known") &mdash; the most common
              symptom in mainland China and other regions with active
              DNS-level blocking. Also fires on captive-portal hijacks
              and broken local resolvers.
            </li>
            <li>
              <strong>Connection refused / timeout</strong> ("Connection
              refused", "Operation timed out") &mdash; DNS resolves but a
              firewall or routing layer drops the traffic. Common in
              corporate networks and TCP-level regional blocks.
            </li>
            <li>
              <strong>TLS handshake failure</strong> ("SSL_connect
              error", "certificate verify failed") &mdash; reached an
              endpoint, but the encrypted handshake didn't complete.
              Usually a TLS-inspecting proxy with a missing CA bundle.
            </li>
            <li>
              <strong>HTTP error</strong> (500-class response) &mdash;
              the API itself is degraded. Check{" "}
              <a
                href="https://status.claude.com"
                target="_blank"
                rel="noopener noreferrer"
              >
                status.claude.com
              </a>
              ; this isn't a network problem.
            </li>
          </ul>
        </section>

        <section id="proxy" className="proto-section">
          <h2>Configure a proxy</h2>
          <p>
            If you have a working VPN, corporate proxy, or other
            outbound network solution, Claudepot uses it via the
            standard environment variables. Set <code>HTTPS_PROXY</code>{" "}
            (or <code>https_proxy</code> / <code>ALL_PROXY</code>)
            before launching:
          </p>
          <pre>
            <code>{`# macOS / Linux
export HTTPS_PROXY=http://127.0.0.1:7890
open -a Claudepot

# Windows (PowerShell)
$env:HTTPS_PROXY = "http://127.0.0.1:7890"
& "$env:LOCALAPPDATA\\Programs\\Claudepot\\Claudepot.exe"`}</code>
          </pre>
          <p>
            macOS and Windows system-wide proxy settings (System
            Settings &rarr; Network &rarr; Proxies, or Settings &rarr;
            Proxy) also work. Settings &rarr; Network in Claudepot
            shows whether a proxy was detected and lets you re-test the
            connection.
          </p>
          <p>
            <strong>What we don't do.</strong> Claudepot doesn't ship
            its own VPN, doesn't recommend specific circumvention
            tools, and doesn't help you set one up. Those are
            jurisdiction-sensitive choices best made with current
            independent sources &mdash; see "What we don't cover" below.
          </p>
        </section>

        <section id="install-mirrors" className="proto-section">
          <h2>Faster CLI install (mirrors)</h2>
          <p>
            The Claude Code CLI is distributed via npm
            (<code>@anthropic-ai/claude-code</code>). The default
            registry (<code>registry.npmjs.org</code>) is reachable from
            most networks but can be slow inside mainland China.
            Configure a faster mirror first:
          </p>
          <pre>
            <code>{`npm config set registry https://registry.npmmirror.com
npm install -g @anthropic-ai/claude-code`}</code>
          </pre>
          <p>
            <code>npmmirror.com</code> is hosted in China by Alibaba and
            is a standard tool every Chinese developer uses; it isn't
            circumvention. To revert:
          </p>
          <pre>
            <code>npm config set registry https://registry.npmjs.org</code>
          </pre>
          <p>
            <strong>Note:</strong> installing the CLI doesn't grant API
            access. The CLI's <code>claude /login</code> flow goes
            through <code>claude.ai</code>, which has its own
            reachability requirements. If you can install the CLI but
            can't authenticate, see the next section.
          </p>
        </section>

        <section id="third-party" className="proto-section">
          <h2>Use a third-party LLM</h2>
          <p>
            If Anthropic itself isn't reachable from your network and
            you don't want to set up a network workaround, route the
            Claude Code CLI through a different provider. Claudepot's
            Third-parties section creates a wrapper binary that points
            the CLI at any OpenAI-compatible endpoint &mdash; including
            providers reachable from mainland China:
          </p>
          <ul>
            <li>
              <strong>DeepSeek</strong> &mdash; strong coding models,
              priced low. Endpoint:{" "}
              <code>https://api.deepseek.com/v1</code>
            </li>
            <li>
              <strong>Kimi (Moonshot)</strong> &mdash; long-context
              models. Endpoint:{" "}
              <code>https://api.moonshot.cn/v1</code>
            </li>
            <li>
              <strong>Qwen (Alibaba DashScope)</strong> &mdash; broad
              model lineup including Qwen-Coder. Endpoint:{" "}
              <code>https://dashscope.aliyuncs.com/compatible-mode/v1</code>
            </li>
            <li>
              <strong>GLM (Zhipu)</strong> &mdash; Chinese-built coding
              and chat models. Endpoint:{" "}
              <code>https://open.bigmodel.cn/api/paas/v4</code>
            </li>
            <li>
              <strong>Ollama (local)</strong> &mdash; run any model on
              your own hardware. Endpoint:{" "}
              <code>http://127.0.0.1:11434/v1</code>
            </li>
          </ul>
          <p>
            In the app: Third-parties &rarr; Add route &rarr; Gateway,
            then pick a preset from the quick-start row. You'll need an
            API key from your chosen provider.
          </p>
        </section>

        <section id="out-of-scope" className="proto-section">
          <h2>What we don't cover</h2>
          <p>
            Setting up a VPN or proxy server is outside the scope of
            this guide. Claudepot doesn't recommend specific
            circumvention tools and doesn't ship or distribute them.
            That choice depends on your jurisdiction, threat model, and
            current conditions &mdash; all of which change faster than
            documentation can track. Independent resources like the{" "}
            <a
              href="https://ooni.org/"
              target="_blank"
              rel="noopener noreferrer"
            >
              OONI project
            </a>{" "}
            track current options for specific regions; consult them
            for current advice.
          </p>
          <p>
            If you've configured a working network path and Claudepot
            still fails, please file an issue at{" "}
            <a
              href="https://github.com/xiaolai/claudepot-app/issues"
              target="_blank"
              rel="noopener noreferrer"
            >
              github.com/xiaolai/claudepot-app
            </a>
            . Include the diagnosis category from the
            in-app panel ("DNS failure", "Connection refused", etc.) so
            we can tell which path is breaking.
          </p>
        </section>
      </div>
    </div>
  );
}
