#!/usr/bin/env python3
"""Build the embedded Claude Code env-var spec from committed evidence.

Two stages, deliberately separated so that *verifying* the shipped artifact
never needs the network or a local Claude Code checkout:

  1. `--rebuild-evidence` (deliberate, non-hermetic) — re-derives
     `crates/claudepot-core/data/cc-env-evidence.json` from
       * the official docs page, cached at dev-docs/env-tools/env-vars-official.md
         (pass --refresh to re-fetch it first),
       * Claude Code's own `utils/managedEnvConstants.ts` (SAFE_ENV_VARS,
         PROVIDER_MANAGED_ENV_VARS, the VERTEX_REGION_CLAUDE_ prefix),
       * `strings` over the installed Claude Code binary, for the
         present-in-build / undocumented-in-build cross-check.

  2. default / `--check` (hermetic, offline) — classifies the committed
     evidence into `crates/claudepot-core/data/cc-env-spec.json`, the artifact
     `claudepot-core` embeds with include_str!. `--check` regenerates into
     memory and compares byte-for-byte, so `cargo xtask verify-docs` can prove
     the shipped artifact is what this script produces from the committed
     evidence. A checksum next to its own artifact would only prove the two
     were edited together.

Provenance is TWO facts, never one. The docs page and the binary cross-check
have different lifetimes — the live docs already list variables absent from any
given binary — so the artifact records `docs_fetched_at` / `docs_sha256`
separately from `binary_crosscheck_version`. Never label the whole artifact
"documented for <version>".

Usage:
    python3 scripts/build-cc-env-spec.py                 # rebuild artifact
    python3 scripts/build-cc-env-spec.py --check         # verify, exit 1 on drift
    python3 scripts/build-cc-env-spec.py --rebuild-evidence [--refresh]
    python3 scripts/build-cc-env-spec.py --html          # dev-docs preview
"""
from __future__ import annotations

import argparse
import datetime
import glob
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
DATA_DIR = os.path.join(ROOT, "crates", "claudepot-core", "data")
TESTDATA_DIR = os.path.join(ROOT, "crates", "claudepot-core", "testdata")
EVIDENCE_JSON = os.path.join(DATA_DIR, "cc-env-evidence.json")
SPEC_JSON = os.path.join(DATA_DIR, "cc-env-spec.json")
VECTORS_JSON = os.path.join(TESTDATA_DIR, "cc-env-vectors.json")

DEV_DOCS = os.path.join(ROOT, "dev-docs")
DOCS_CACHE = os.path.join(DEV_DOCS, "env-tools", "env-vars-official.md")
OUT_HTML = os.path.join(DEV_DOCS, "artifacts", "claude-code-env-vars.html")
DOCS_URL = "https://code.claude.com/docs/en/env-vars.md"
CC_SRC_CONSTANTS = os.path.expanduser(
    "~/github/claude_code_src/src/utils/managedEnvConstants.ts"
)

# ---------------------------------------------------------------------------
# Classification tables. Everything here is evidence, not inference: each entry
# is either lifted from Claude Code's own source or adjudicated by hand against
# the variable's official prose. Where a rule could only guess, it emits an
# explicit "unknown" rather than a confident-sounding label.
# ---------------------------------------------------------------------------

# Vars that are host/subprocess-injected, not user-set. Rendered read-only.
HOST_INJECTED = {
    "CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_ACCOUNT_UUID", "CLAUDE_CODE_ORGANIZATION_UUID",
    "CLAUDE_CODE_REMOTE", "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_ACCOUNT_TAGGED_ID", "CLAUDE_CODE_VERSION",
    # "Set by Claude Code, not by you" — both are exported per session to
    # hooks and Bash commands when the session binds its inbox socket.
    # Editing either in a settings `env` block would overwrite a value CC
    # derives at runtime.
    "CLAUDE_CODE_MESSAGING_TOKEN", "CLAUDE_CODE_MESSAGING_SOCKET",
}

# Vars we refuse to edit here because writing them splits Claude Code's own
# bootstrap. CC resolves its config directory from process.env BEFORE settings
# load (utils/env.ts:25 getGlobalClaudeFile, utils/envUtils.ts:7
# getClaudeConfigHomeDir — both memoized on the env value), then applies
# settings.env over it, so some paths resolve against the old directory and
# later ones against the new. Claudepot's own target file does not move:
# paths.rs:5 reads the same variable from ITS process env.
#
# The audit for siblings of this class covered every documented row whose name
# or prose touches config location: CLAUDE_CONFIG_DIR is the only one. The
# nearest neighbour, CLAUDE_ENV_FILE, is read per-Bash-command rather than at
# bootstrap, so it is editable and carries an execute-code hazard instead.
BOOTSTRAP_SPLIT_BRAIN = {"CLAUDE_CONFIG_DIR"}

# Vars Claude Code reads ONLY from the process environment it was started
# with, and explicitly NOT from a settings file `env` block — which is the
# block this pane writes. Editing one here would land a key in
# settings.json, report success, and change nothing: the same silent
# no-op class as writing a `cleanupPeriodDays` value CC rejects.
#
# The docs state it outright for CLAUDE_CODE_PROJECT_DIR_NAME: Claude Code
# "reads it only from the environment you start `claude` from, never from a
# settings file `env` block". This set is for that sentence, and is checked
# against the prose below so a row cannot drift out of it silently.
ENV_ONLY_NOT_SETTINGS = {"CLAUDE_CODE_PROJECT_DIR_NAME"}

# The phrasing the docs use for the rule above. Any documented row carrying
# it must be in ENV_ONLY_NOT_SETTINGS — otherwise a variable CC ignores from
# settings would render as an editable field. Checked at build time rather
# than trusted, because the failure is silent by construction.
#
# The docs wrap the phrase in a markdown link —
# "never from a [settings file `env` block](#in-settings-files)" — so the
# pattern has to tolerate the brackets and backticks between the words. A
# first version anchored on the bare words and matched nothing, which the
# both-directions check below caught immediately.
ENV_ONLY_PROSE_RE = re.compile(
    r"never\s+from\s+a\s+\[?settings\s+file\s+`?env`?\s+block", re.IGNORECASE
)

NUM_SUFFIX = ("_MS", "_TOKENS", "_LENGTH", "_SECONDS", "_MAX", "_CAP",
              "_SIZE", "_DEPTH", "_BATCH_SIZE", "_TTL", "_COUNT", "_INTERVAL",
              "_LIMIT", "_CEILING", "_THRESHOLD", "_WINDOW", "_PORT")

# A default that belongs to THIS variable. Every form requires a connector
# (`to`/`is`/`:`), a parenthesis, or a backtick around the number, so
# "instead of the default 5 minutes" (ENABLE_PROMPT_CACHING_1H, a plain
# toggle, whose 5 belongs to the prompt cache and not to the variable) no
# longer reads as a numeric default and no longer drags the variable into
# the number branch. The bare-backtick form is the docs' own convention:
# "Default `20000` (20 seconds)".
OWN_DEFAULT_RE = re.compile(
    r"\(default:?\s*`?(\d[\d_,]*)"
    r"|[Dd]efaults?\s+(?:to|is)\s+`?(\d[\d_,]*)"
    r"|[Dd]efaults?:\s*`?(\d[\d_,]*)"
    r"|[Dd]efaults?\s+`(\d[\d_,]*)`"
)
OWN_DEFAULT_URL_RE = re.compile(r"[Dd]efaults?\s+to\s+`(https?://[^`]+)`")

# Names that LOOK credential-bearing. Not a classifier — an auditor. Every
# documented row it matches must appear in exactly one of the two curated sets
# below, so a newly documented `*_API_KEY` fails the build until adjudicated.
SECRET_NAME_HINT = re.compile(
    r"API_KEY|AUTH_TOKEN|_TOKEN$|_TOKEN_|BEARER|SECRET|"
    r"SIGNING_KEY|PASSPHRASE|CLIENT_KEY|CREDENTIAL"
)

# `secret: true` means "the value may carry a credential, so never serialize
# it". Adjudicated one by one against the official prose.
SECRET_ENV_VARS = {
    "ANTHROPIC_API_KEY",              # "API key sent as X-Api-Key header"
    "ANTHROPIC_AUTH_TOKEN",           # bearer token
    "ANTHROPIC_AWS_API_KEY",          # "Workspace API key ... sent as x-api-key"
    "ANTHROPIC_FOUNDRY_API_KEY",      # Foundry API key
    "ANTHROPIC_FOUNDRY_AUTH_TOKEN",   # Foundry bearer token
    "AWS_BEARER_TOKEN_BEDROCK",       # Bedrock bearer token
    "CLAUDE_CODE_CLIENT_KEY",         # mTLS client key
    "CLAUDE_CODE_CLIENT_KEY_PASSPHRASE",
    "CLAUDE_CODE_OAUTH_REFRESH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "MCP_CLIENT_SECRET",
    # The peer-messaging `peerToken`. CC exports it per session to hooks and
    # Bash commands, and a client proves session membership by opening with
    # `{"type":"auth","token":"<token>"}` — on native Windows CC *requires*
    # that line. So it is a bearer credential for the one channel that can
    # inject a prompt into a running session (see AGENTS.md "Peer
    # messaging"), and must never be serialized.
    "CLAUDE_CODE_MESSAGING_TOKEN",
    # Not name-matched, and the whole reason CC's SAFE_ENV_VARS cannot double
    # as a disclosure judgement: CC lists this as pre-trust-safe and its
    # documented format is `Name: Value`, which happily holds
    # `Authorization: Bearer ...`.
    "ANTHROPIC_CUSTOM_HEADERS",
}

# Name-matched but NOT secret. Each needs a reason, so the exemption is a
# decision rather than an oversight.
NOT_SECRET_DESPITE_NAME = {
    # A refresh interval in milliseconds, not a key. Also in CC's
    # SAFE_ENV_VARS, which a pure regex would have contradicted.
    "CLAUDE_CODE_API_KEY_HELPER_TTL_MS": "interval in ms, not a credential",
}

# Hazard taxonomy. The first three groups are Claude Code's own, quoted from
# the comment above SAFE_ENV_VARS in utils/managedEnvConstants.ts. The last two
# are Claudepot's, for classes CC does not enumerate.
HAZARD_REDIRECT = {  # "REDIRECT TO ATTACKER-CONTROLLED SERVER"
    "ANTHROPIC_BASE_URL", "ANTHROPIC_BEDROCK_BASE_URL",
    "ANTHROPIC_FOUNDRY_BASE_URL", "ANTHROPIC_VERTEX_BASE_URL",
    "HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY",
    "http_proxy", "https_proxy", "no_proxy",
    "OTEL_EXPORTER_OTLP_ENDPOINT", "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
    "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
}
HAZARD_TRUST_CERT = {  # "TRUST ATTACKER-CONTROLLED SERVER"
    "NODE_TLS_REJECT_UNAUTHORIZED", "NODE_EXTRA_CA_CERTS",
}
HAZARD_SWITCH_PROJECT = {  # "SWITCH TO ATTACKER-CONTROLLED PROJECT"
    "ANTHROPIC_FOUNDRY_RESOURCE", "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN",
    "AWS_BEARER_TOKEN_BEDROCK",
}
# Claudepot's addition: values that end up in a spawned command line.
HAZARD_EXECUTE_CODE = {
    "CLAUDE_CODE_SHELL_PREFIX", "CLAUDE_ENV_FILE", "CLAUDE_CODE_SHELL",
    "CLAUDE_CODE_GIT_BASH_PATH",
}
# Claudepot's addition: silently stops security updates from landing.
HAZARD_DISABLE_UPDATES = {"DISABLE_AUTOUPDATER", "DISABLE_UPDATES"}

CC_DERIVED_HAZARDS = HAZARD_REDIRECT | HAZARD_TRUST_CERT | HAZARD_SWITCH_PROJECT

# Enums where the docs row genuinely defers to another page. Applied to the
# spec, and asserted to be a superset of anything parseable from the prose.
#
# Currently empty: CLAUDE_CODE_EFFORT_LEVEL used to live here with four of its
# six values, which is exactly the drift the superset assertion now catches —
# its own prose lists all six, so it is derived rather than overridden.
ENUM_OVERRIDES: dict[str, list[str]] = {}

# Curated enums for variables the docs page mentions only in prose, outside the
# `## Variables` table, and therefore does not produce a row for. Parked, not
# applied — asserted to contain no documented row, so the day one of these
# becomes a real row the build fails and the value moves to ENUM_OVERRIDES.
PARKED_ENUM_OVERRIDES = {
    "OTEL_METRICS_EXPORTER": ["otlp", "prometheus", "console"],
    "OTEL_LOGS_EXPORTER": ["otlp", "console"],
    "OTEL_TRACES_EXPORTER": ["otlp", "console"],
}

CATEGORIES = [
    ("auth", "Auth & Providers"), ("model", "Models"),
    ("limit", "Limits & Timeouts"), ("agent", "Subagents, Workflows & Tasks"),
    ("plugin", "Plugins, Skills & MCP"), ("memory", "Memory"),
    ("session", "Session & Host"), ("network", "Network & Proxy"),
    ("telemetry", "Telemetry & Logging"), ("ui", "UI & Terminal"),
    ("misc", "Other"),
]


def categorize(name: str) -> str:
    n = name

    def has(*s):
        return any(x in n for x in s)

    if has("OTEL", "TELEMETRY", "METRICS", "DATADOG", "GROWTHBOOK",
           "OTLP", "TRACE"):
        return "telemetry"
    if has("MODEL", "HAIKU", "SONNET", "OPUS", "FABLE") and "USE_" not in n:
        return "model"
    if has("API_KEY", "_AUTH", "OAUTH", "BEDROCK", "VERTEX", "FOUNDRY",
           "MANTLE", "GATEWAY", "BEARER", "CERT", "CLIENT_KEY", "AWS",
           "WORKSPACE_ID", "SERVICE_TIER", "ANTHROPIC_MODEL", "CUSTOM_MODEL",
           "FEDERATION", "IDENTITY"):
        return "auth"
    if has("MEMORY"):
        return "memory"
    if has("PLUGIN", "SKILL", "MCP", "MARKETPLACE", "ARTIFACT"):
        return "plugin"
    if has("SUBAGENT", "AGENT", "WORKFLOW", "TASK", "COORDINATOR", "TEAM",
           "FLEET", "ASYNC_AGENT", "BACKGROUND"):
        return "agent"
    if has("PROXY", "HTTP", "SOCKS", "TLS", "SSE", "WEBSOCKET", "_PORT",
           "GZIP", "TIMEOUT_MS", "CONNECT"):
        if has("PROXY", "HTTP", "SOCKS", "TLS", "WEBSOCKET", "GZIP"):
            return "network"
    if has("MAX_", "_TOKENS", "TIMEOUT", "THRESHOLD", "_LIMIT", "RETRIES",
           "CONCURRENCY", "DEADLINE", "_WAIT", "TTL", "IDLE", "_MS",
           "_SECONDS", "PCT", "WINDOW", "CEILING"):
        return "limit"
    if has("SESSION", "RESUME", "ACCOUNT", "ORGANIZATION", "REMOTE",
           "ENTRYPOINT", "CONTAINER", "HOST", "CHILD", "CLAUDECODE"):
        return "session"
    if has("MOUSE", "COLOR", "CURSOR", "SCREEN", "FLICKER", "SCROLL",
           "SYNTAX", "TERMINAL", "TITLE", "TMUX", "REPAINT", "ALT_SCREEN",
           "ACCESSIBILITY", "AX_", "FULLSCREEN", "AFK", "NATIVE", "MENU"):
        return "ui"
    return "misc"


def humanize_num(s: str) -> str:
    return s.replace("_", "").replace(",", "")


def extract_default(purpose: str) -> str:
    m = OWN_DEFAULT_RE.search(purpose)
    if m:
        return humanize_num(next(g for g in m.groups() if g))
    m = OWN_DEFAULT_URL_RE.search(purpose)
    return m.group(1) if m else ""


# A literal a select could offer verbatim.
LITERAL_VALUE_RE = re.compile(r"[a-z0-9][a-z0-9._-]{0,15}\Z")
# The docs' own enumeration form: "Values: `a`, `b`, or `c`."
VALUES_CLAUSE_RE = re.compile(r"Values?:\s*(.+?)(?:\.\s|\.$|$)", re.S)


def extract_enum(purpose: str):
    """Closed value sets stated by the prose, or None.

    Bails whenever the clause contains a backticked token that a select could
    not offer verbatim — `auto:N` in ENABLE_TOOL_SEARCH is the case that
    matters: closing that set would reject the documented `auto:5`.
    """
    m = VALUES_CLAUSE_RE.search(purpose)
    if m:
        toks = re.findall(r"`([^`]+)`", m.group(1))
        if toks and all(LITERAL_VALUE_RE.fullmatch(t) for t in toks):
            seen, vals = set(), []
            for t in toks:
                if t not in seen:
                    seen.add(t)
                    vals.append(t)
            if len(vals) >= 2:
                return vals
        return None
    # Fallback: a parenthetical list, e.g. "(`default`, `flex`, `priority`)".
    for m in re.finditer(r"\(([^()]*`[^`]+`[^()]*)\)", purpose):
        toks = re.findall(r"`([^`]+)`", m.group(1))
        vals = [t for t in toks if LITERAL_VALUE_RE.fullmatch(t)]
        if len(vals) != len(toks):
            continue
        if len(vals) >= 2:
            return vals
    return None


def numeric_signal(name: str, purpose: str):
    """Positive evidence that the VALUE is a number, or None.

    Runs BEFORE the "Set to `0`/`1`" toggle test. "Set to `0` to disable the
    cap" is standard prose for a number whose zero is special, and the old
    precedence turned five token budgets and timeouts into switches that would
    have written `1` into a millisecond field.
    """
    if name.endswith(NUM_SUFFIX):
        return "name-suffix"
    if re.search(r"\bmillisecond", purpose):
        return "prose-milliseconds"
    if re.search(r"percentage \(1-?100\)", purpose):
        return "prose-percentage"
    if OWN_DEFAULT_RE.search(purpose):
        return "prose-own-default"
    return None


def text_format(name: str, purpose: str, secret: bool) -> str:
    """A display HINT, never validation. `*MODEL*` catches
    ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION, a prose string — validating
    against this would reject legitimate values."""
    if secret:
        return "secret"
    if "URL" in name or "BASE_URL" in name:
        return "url"
    if "MODEL" in name:
        return "model-id"
    if name.endswith(("_DIR", "_PATH", "_FILE", "_STORE")) or "PATH" in name:
        return "path"
    if "HEADER" in name:
        return "header-list"
    if "comma-separated" in purpose or "newline-separated" in purpose:
        return "list"
    return "text"


def unit_for(name: str, purpose: str) -> str:
    if name.endswith("_MS") or "millisecond" in purpose:
        return "ms"
    if name.endswith("_SECONDS"):
        return "s"
    if name.endswith("_TOKENS"):
        return "tokens"
    if name.endswith(("_LENGTH", "_MAX_OUTPUT_LENGTH")):
        return "chars"
    if re.search(r"percentage \(1-?100\)", purpose):
        return "%"
    return ""


def safety_for(name: str, safe_set: set, provider_set: set,
               provider_prefixes: list) -> dict:
    upper = name.upper()
    hazards = []
    if name in HAZARD_REDIRECT:
        hazards.append("redirect")
    if name in HAZARD_TRUST_CERT:
        hazards.append("trust_cert")
    if name in HAZARD_EXECUTE_CODE:
        hazards.append("execute_code")
    if name in HAZARD_SWITCH_PROJECT:
        hazards.append("switch_project")
    if name in HAZARD_DISABLE_UPDATES:
        hazards.append("disable_updates")

    pretrust_safe = upper in safe_set
    # Absence from SAFE_ENV_VARS says something is risky without saying what.
    # Naming a specific risk here would be the same sin as guessing a control
    # type, so the unestablished case is labelled as unestablished.
    if not hazards and not pretrust_safe:
        hazards.append("unknown")

    blocked = None
    if name in BOOTSTRAP_SPLIT_BRAIN:
        blocked = "bootstrap_split_brain"
    elif name in HOST_INJECTED:
        blocked = "host_injected"
    elif name in ENV_ONLY_NOT_SETTINGS:
        blocked = "env_only_not_settings"

    return {
        "secret": name in SECRET_ENV_VARS,
        "blocked_reason": blocked,
        "pretrust_safe": pretrust_safe,
        "provider_managed": upper in provider_set
        or any(upper.startswith(p) for p in provider_prefixes),
        "hazards": hazards,
    }


# The two vocabularies Claude Code's docs use for a boolean. Both work at
# runtime — `isEnvTruthy` accepts `1|true|yes|on` — but the pane offers the
# literal the variable's own documentation states, because offering `1` where
# the doc says `true` is guessing at a value, which is the one thing this
# generator refuses to do anywhere else.
TOGGLE_VOCABULARIES = (("1", "0"), ("true", "false"))


def toggle_vocabulary(purpose: str):
    """Which on/off literal pair this variable's prose uses, or None.

    Eleven documented variables state "Set to true" / "Set to false" in the
    docs' backticked form and used to fall through to free text — a boolean
    rendered as an open text box, inviting values Claude Code reads as false.
    """
    for on, off in TOGGLE_VOCABULARIES:
        if f"Set to `{on}`" in purpose or f"Set to `{off}`" in purpose:
            return on, off
    return None


def classify(name: str, purpose: str, safety: dict) -> dict:
    """Precedence: explicit override > number > toggle > prose enum > text.

    Number beats toggle (rev 3 §2.1). Toggle still beats the prose enum, so
    backticked field names inside a toggle's prose are not misread as values.
    """
    override = ENUM_OVERRIDES.get(name)
    prose_enum = extract_enum(purpose)
    num = numeric_signal(name, purpose)
    vocab = toggle_vocabulary(purpose)

    vals = None
    if override:
        control, vals = "enum", list(override)
    elif num:
        control = "number"
    elif vocab:
        control = "toggle"
    elif prose_enum:
        control, vals = "enum", prose_enum
    else:
        control = "text"

    on = off = None
    if control == "toggle":
        on, off_literal = vocab
        # `off = "unset"` means the variable has no documented off VALUE:
        # turning it off is removing the key. Only when the prose states the
        # off literal is there a third, distinct state to offer.
        off = off_literal if f"Set to `{off_literal}`" in purpose else "unset"
        vals = [off, on] if off != "unset" else [on]

    return {
        "control": control,
        "values": vals,
        "default": extract_default(purpose),
        "unit": unit_for(name, purpose) if control == "number" else "",
        "on": on,
        "off": off,
        "numeric_evidence": num or "",
        "format": text_format(name, purpose, safety["secret"])
        if control == "text"
        else "",
    }


# ---------------------------------------------------------------------------
# Assertions — drift fails loudly rather than shipping a wrong control.
# ---------------------------------------------------------------------------

def assert_tables(rows: list, safe_set: set) -> None:
    names = {r["name"] for r in rows}
    problems = []

    for name in sorted(names):
        if not SECRET_NAME_HINT.search(name):
            continue
        if name in SECRET_ENV_VARS or name in NOT_SECRET_DESPITE_NAME:
            continue
        problems.append(
            f"{name}: name reads as credential-bearing but is in neither "
            f"SECRET_ENV_VARS nor NOT_SECRET_DESPITE_NAME — adjudicate it"
        )
    for name in sorted(SECRET_ENV_VARS - names):
        problems.append(f"{name}: in SECRET_ENV_VARS but not a documented row")
    for name in sorted(set(NOT_SECRET_DESPITE_NAME) - names):
        problems.append(
            f"{name}: in NOT_SECRET_DESPITE_NAME but not a documented row"
        )

    by_name = {r["name"]: r for r in rows}
    for name, vals in sorted(ENUM_OVERRIDES.items()):
        if name not in by_name:
            problems.append(
                f"{name}: in ENUM_OVERRIDES but not a documented row"
            )
            continue
        prose = set(extract_enum(by_name[name]["doc"]) or [])
        missing = prose - set(vals)
        if missing:
            problems.append(
                f"{name}: ENUM_OVERRIDES is missing prose values "
                f"{sorted(missing)} — an override must be a superset"
            )
    for name in sorted(set(PARKED_ENUM_OVERRIDES) & names):
        problems.append(
            f"{name}: parked in PARKED_ENUM_OVERRIDES but IS a documented row "
            f"now — move it to ENUM_OVERRIDES"
        )

    for r in rows:
        used = [
            (on, off)
            for on, off in TOGGLE_VOCABULARIES
            if f"Set to `{on}`" in r["doc"] or f"Set to `{off}`" in r["doc"]
        ]
        if len(used) > 1:
            problems.append(
                f"{r['name']}: prose uses both the 1/0 and true/false toggle "
                f"vocabularies — adjudicate which literal the pane should offer"
            )

    # A variable CC ignores from a settings `env` block must never render as
    # an editable field here — writing it would report success and change
    # nothing. Detect the docs' own phrasing rather than trusting the curated
    # set to stay complete, in both directions: an unlisted row carrying the
    # sentence fails, and a listed row that lost it fails too, so the set
    # cannot outlive its evidence.
    for r in rows:
        says_env_only = bool(ENV_ONLY_PROSE_RE.search(r["doc"]))
        listed = r["name"] in ENV_ONLY_NOT_SETTINGS
        if says_env_only and not listed:
            problems.append(
                f"{r['name']}: docs say CC never reads it from a settings "
                f"`env` block, but it is not in ENV_ONLY_NOT_SETTINGS — it "
                f"would render as an editable field that silently does nothing"
            )
        if listed and not says_env_only:
            problems.append(
                f"{r['name']}: in ENV_ONLY_NOT_SETTINGS but the docs no longer "
                f"say so — re-read the prose and drop it if CC now honours it"
            )

    leaked = sorted(n for n in CC_DERIVED_HAZARDS if n.upper() in safe_set)
    for name in leaked:
        problems.append(
            f"{name}: listed in CC's dangerous taxonomy AND in SAFE_ENV_VARS — "
            f"the extracted lists disagree, re-read managedEnvConstants.ts"
        )

    if problems:
        sys.exit("Classification table drift:\n  " + "\n  ".join(problems))


# ---------------------------------------------------------------------------
# Evidence (stage 1)
# ---------------------------------------------------------------------------

def ensure_docs(refresh: bool) -> str:
    if refresh or not os.path.exists(DOCS_CACHE):
        try:
            data = subprocess.run(
                ["curl", "-sL", "--max-time", "30", DOCS_URL],
                capture_output=True, text=True, check=True).stdout
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            if not os.path.exists(DOCS_CACHE):
                sys.exit(f"Could not fetch official docs and no cache: {e}")
            print(f"warn: could not refresh docs ({e}); using cache",
                  file=sys.stderr)
        else:
            if "## Variables" not in data:
                if not os.path.exists(DOCS_CACHE):
                    sys.exit("Fetched docs did not contain the Variables table.")
                print("warn: fetched docs missing table; using cache",
                      file=sys.stderr)
            else:
                os.makedirs(os.path.dirname(DOCS_CACHE), exist_ok=True)
                with open(DOCS_CACHE, "w", encoding="utf-8") as f:
                    f.write(data)
    with open(DOCS_CACHE, encoding="utf-8") as f:
        return f.read()


def parse_docs(md: str):
    rows, in_table = [], False
    for ln in md.splitlines():
        if ln.startswith("## Variables"):
            in_table = True
            continue
        if ln.startswith("## ") and in_table:
            break
        # Only rows inside the Variables section. `in_table` was computed and
        # then ignored, so a table anywhere earlier in the page would have
        # been scraped into the spec as if it listed environment variables.
        if not in_table:
            continue
        # Case-sensitive: the docs list only SCREAMING_CASE names today, but
        # the proxy family (`http_proxy`) is lowercase by convention, and an
        # uppercase-only pattern would drop such a row silently rather than
        # noisily.
        m = re.match(r"\|\s*`([A-Za-z_][A-Za-z0-9_]*)`\s*\|\s*(.*?)\s*\|\s*$", ln)
        if m:
            purpose = re.sub(r"\{/\*.*?\*/\}", "", m.group(2)).strip()
            rows.append({"name": m.group(1), "doc": purpose})
    return rows


def _safety_source_kind() -> str:
    """`pinned_mirror` while the safety lists come from the abandoned
    third-party source checkout; `installed_binary` once a future
    extraction reads them from the running Claude Code.

    Derived from the path actually opened rather than declared, so it
    cannot disagree with what was read.
    """
    return ("pinned_mirror" if "claude_code_src" in CC_SRC_CONSTANTS
            else "installed_binary")


def _safety_source_version() -> str:
    """The version the safety source is stuck at.

    Read from the published tarball sitting in the mirror checkout
    (`claude-code-<version>.tgz`) so the number is evidence rather than a
    literal someone maintains by hand. `unknown` when it cannot be
    determined — an honest blank beats a stale constant, and the consumer
    renders it as-is.
    """
    root = CC_SRC_CONSTANTS.split("/src/")[0]
    try:
        for name in sorted(os.listdir(root)):
            m = re.fullmatch(r"claude-code-(\d+\.\d+\.\d+)\.tgz", name)
            if m:
                return m.group(1)
    except OSError:
        pass
    return "unknown"


def _ts_set(src: str, ident: str) -> list:
    """Extract a `new Set([...])` literal from TypeScript. Comments are
    stripped first: PROVIDER_MANAGED_ENV_VARS' own comments contain
    apostrophes, which a naive quoted-string scan reads as members."""
    m = re.search(ident + r"\s*=\s*new Set\(\[(.*?)\]\)", src, re.S)
    if not m:
        sys.exit(f"Could not find {ident} in {CC_SRC_CONSTANTS}")
    body = re.sub(r"//[^\n]*", "", m.group(1))
    return sorted(set(re.findall(r"'([^']+)'", body)))


def _ts_prefixes(src: str, ident: str) -> list:
    m = re.search(ident + r"\s*=\s*\[(.*?)\]", src, re.S)
    if not m:
        sys.exit(f"Could not find {ident} in {CC_SRC_CONSTANTS}")
    body = re.sub(r"//[^\n]*", "", m.group(1))
    return sorted(set(re.findall(r"'([^']+)'", body)))


def find_binary():
    env = os.environ.get("CLAUDE_BIN")
    if env and os.path.exists(env):
        return env
    exe = shutil.which("claude")
    if exe and os.path.exists(os.path.realpath(exe)):
        return os.path.realpath(exe)
    cands = [c for c in glob.glob(os.path.expanduser(
        "~/.local/share/claude/versions/*")) if os.path.isfile(c)]
    if cands:
        # Sort by version tuple, not lexicographically: "2.1.99" sorts AFTER
        # "2.1.100" as a string, so the plain sort picked an older binary the
        # moment a patch number passed 99.
        def version_key(path):
            m = re.search(r"(\d+)\.(\d+)\.(\d+)", os.path.basename(path))
            return tuple(int(g) for g in m.groups()) if m else (0, 0, 0)

        return sorted(cands, key=version_key)[-1]
    for p in ("~/.claude/local/node_modules/@anthropic-ai/claude-code/cli.js",
              "/opt/homebrew/lib/node_modules/@anthropic-ai/claude-code/cli.js",
              "/usr/local/lib/node_modules/@anthropic-ai/claude-code/cli.js"):
        if os.path.exists(os.path.expanduser(p)):
            return os.path.expanduser(p)
    return None


def version_of(binary: str) -> str:
    m = re.search(r"\d+\.\d+\.\d+", os.path.basename(binary))
    if m:
        return m.group(0)
    m = re.search(r"versions/(\d+\.\d+\.\d+)", os.path.realpath(binary))
    return m.group(1) if m else "unknown"


def rebuild_evidence(refresh: bool) -> dict:
    md = ensure_docs(refresh)
    rows = parse_docs(md)
    if not rows:
        sys.exit("Parsed 0 variables from the docs — table format changed?")

    if not os.path.exists(CC_SRC_CONSTANTS):
        sys.exit(f"Claude Code source not found at {CC_SRC_CONSTANTS}. "
                 "Evidence rebuild needs it; --check does not.")
    with open(CC_SRC_CONSTANTS, encoding="utf-8") as f:
        cc_src = f.read()

    binary = find_binary()
    if not binary:
        sys.exit("No Claude Code binary found for the cross-check.")
    version = version_of(binary)
    # `check=True` and a non-empty assertion: without them a missing
    # `strings`, an unreadable binary, or a stripped artifact yields empty
    # output, and the run would still write an authoritative-looking
    # `present_in_build` / `undocumented_in_build` saying every documented
    # variable is absent from your build.
    try:
        proc = subprocess.run(["strings", "-n", "6", binary],
                              capture_output=True, text=True,
                              errors="ignore", check=True)
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        sys.exit(f"Could not scan {binary} with `strings`: {e}")
    raw = proc.stdout
    if not raw.strip():
        sys.exit(f"`strings` produced no output for {binary} — refusing to "
                 f"record an empty binary cross-check as a finding.")
    # Keep exactly what the two consumers need, and nothing else: the
    # documented rows (for `present_in_build`) plus every CC-namespaced name
    # (for `undocumented_in_build`). Filtering by a hand-written prefix list
    # instead would silently mark unprefixed documented rows — NO_PROXY,
    # BASH_MAX_TIMEOUT_MS — as absent from a build that contains them.
    doc_names = {r["name"] for r in rows}
    found = set(re.findall(r"[A-Z][A-Z0-9_]{2,}", raw))
    # `strings` also finds the *prefixes* — the bundle concatenates names, so
    # bare `ANTHROPIC_` and `CLAUDE_CODE_` show up as if they were variables.
    # The undocumented list is rendered to users as "names found in your
    # binary"; a truncated fragment presented that way is a confident-sounding
    # lie, so a name must have a segment after its last underscore.
    plausible = {n for n in found if not n.endswith("_")}
    binary_names = sorted(
        (plausible & doc_names)
        | {n for n in plausible if n.startswith(("CLAUDE_CODE_", "ANTHROPIC_"))}
    )

    # The date the cached page was last written, not "today" — without
    # --refresh nothing was fetched, and stamping today's date on last
    # month's cache is the kind of quiet lie the provenance fields exist
    # to prevent.
    fetched_at = datetime.date.fromtimestamp(
        os.path.getmtime(DOCS_CACHE)).isoformat()

    return {
        "docs_url": DOCS_URL,
        "docs_fetched_at": fetched_at,
        "docs_sha256": hashlib.sha256(md.encode("utf-8")).hexdigest(),
        "rows": rows,
        # The Claude Code SOURCE checkout the safety lists were read from,
        # which is not necessarily the installed binary's version. Recorded
        # as the file's own mtime date rather than borrowing the binary's
        # number — pairing a source date with a binary version would be the
        # same kind of quiet lie the two provenance fields exist to prevent.
        "cc_source_read_at": datetime.date.fromtimestamp(
            os.path.getmtime(CC_SRC_CONSTANTS)).isoformat(),
        # WHICH KIND of source the safety lists came from, recorded in the
        # artifact rather than hardcoded in the Rust that renders it. The
        # consumer discloses this to the user as provenance, and a
        # constant on that side would go on asserting "pinned mirror" as
        # fact after this generator moved to a different source — the
        # exact status-surface-states-an-unverified-claim failure the
        # disclosure exists to prevent. Change the source and the artifact
        # says so on the next rebuild.
        "cc_source_kind": _safety_source_kind(),
        "cc_source_version": _safety_source_version(),
        "cc_safe_env_vars": _ts_set(cc_src, "SAFE_ENV_VARS"),
        "cc_provider_managed_env_vars": _ts_set(
            cc_src, "PROVIDER_MANAGED_ENV_VARS"),
        "cc_provider_managed_prefixes": _ts_prefixes(
            cc_src, "PROVIDER_MANAGED_ENV_PREFIXES"),
        "binary_crosscheck_version": version,
        "binary_env_names": binary_names,
    }


# ---------------------------------------------------------------------------
# Spec (stage 2) — a pure function of the committed evidence
# ---------------------------------------------------------------------------

def build_spec(ev: dict) -> dict:
    safe_set = {n.upper() for n in ev["cc_safe_env_vars"]}
    provider_set = {n.upper() for n in ev["cc_provider_managed_env_vars"]}
    provider_prefixes = [p.upper() for p in ev["cc_provider_managed_prefixes"]]
    binset = set(ev["binary_env_names"])

    assert_tables(ev["rows"], safe_set)

    vars_out = []
    for row in ev["rows"]:
        name, purpose = row["name"], row["doc"]
        safety = safety_for(name, safe_set, provider_set, provider_prefixes)
        spec = classify(name, purpose, safety)
        vars_out.append({
            "name": name,
            "category": categorize(name),
            "doc": purpose,
            "present_in_build": name in binset,
            "safety": safety,
            **spec,
        })

    doc_names = {r["name"] for r in ev["rows"]}
    cc_bin = {n for n in binset if n.startswith(("CLAUDE_CODE_", "ANTHROPIC_"))}

    return {
        "schema_version": 1,
        "docs_url": ev["docs_url"],
        "docs_fetched_at": ev["docs_fetched_at"],
        "docs_sha256": ev["docs_sha256"],
        "binary_crosscheck_version": ev["binary_crosscheck_version"],
        "cc_source_read_at": ev["cc_source_read_at"],
        "cc_source_kind": ev["cc_source_kind"],
        "cc_source_version": ev["cc_source_version"],
        # Shipped so the renderer groups by the generator's own order and
        # labels instead of hand-copying this table into TypeScript, where
        # it would drift the first time a category is added or reordered.
        "categories": [{"key": k, "label": lbl} for k, lbl in CATEGORIES],
        "documented_count": len(vars_out),
        "undocumented_in_build": sorted(cc_bin - doc_names),
        "documented_not_in_build": sorted(doc_names - binset),
        "vars": vars_out,
    }


def serialize(spec: dict) -> bytes:
    """Minified, key-sorted, newline-terminated. Byte-for-byte stable so
    `--check` compares bytes rather than a re-parse."""
    return (json.dumps(spec, ensure_ascii=False, sort_keys=True,
                       separators=(",", ":")) + "\n").encode("utf-8")


def check_vectors(spec: dict) -> list:
    """Golden vectors are hand-authored expectations, not generator output —
    a generated vector would only lock in whatever the code already says."""
    if not os.path.exists(VECTORS_JSON):
        return [f"missing {VECTORS_JSON}"]
    with open(VECTORS_JSON, encoding="utf-8") as f:
        vectors = json.load(f)
    by = {v["name"]: v for v in spec["vars"]}
    problems = []
    for vec in vectors["vectors"]:
        actual = by.get(vec["name"])
        if actual is None:
            problems.append(f"{vec['name']}: no such documented variable")
            continue
        for key, want in vec.items():
            if key in ("name", "why"):
                continue
            if key == "safety":
                for skey, swant in want.items():
                    got = actual["safety"].get(skey)
                    if got != swant:
                        problems.append(
                            f"{vec['name']}.safety.{skey}: want {swant!r}, "
                            f"got {got!r}")
                continue
            got = actual.get(key)
            if got != want:
                problems.append(
                    f"{vec['name']}.{key}: want {want!r}, got {got!r}")
    return problems


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--rebuild-evidence", action="store_true",
                    help="re-derive the committed evidence (needs network / "
                         "CC source / installed binary)")
    ap.add_argument("--refresh", action="store_true",
                    help="with --rebuild-evidence, re-fetch the docs page")
    ap.add_argument("--check", action="store_true",
                    help="verify the committed artifact byte-for-byte")
    ap.add_argument("--html", action="store_true",
                    help="also write the dev-docs preview")
    args = ap.parse_args()

    if args.rebuild_evidence:
        ev = rebuild_evidence(args.refresh)
        os.makedirs(DATA_DIR, exist_ok=True)
        with open(EVIDENCE_JSON, "w", encoding="utf-8") as f:
            json.dump(ev, f, ensure_ascii=False, sort_keys=True, indent=1)
            f.write("\n")
        print(f"evidence  : {EVIDENCE_JSON} ({len(ev['rows'])} rows, "
              f"docs {ev['docs_fetched_at']}, "
              f"binary {ev['binary_crosscheck_version']})")

    if not os.path.exists(EVIDENCE_JSON):
        sys.exit(f"No committed evidence at {EVIDENCE_JSON}. "
                 "Run with --rebuild-evidence first.")
    with open(EVIDENCE_JSON, encoding="utf-8") as f:
        ev = json.load(f)

    spec = build_spec(ev)
    payload = serialize(spec)

    vector_problems = check_vectors(spec)

    if args.check:
        problems = list(vector_problems)
        if not os.path.exists(SPEC_JSON):
            problems.append(f"missing artifact {SPEC_JSON}")
        else:
            with open(SPEC_JSON, "rb") as f:
                on_disk = f.read()
            if on_disk != payload:
                problems.append(
                    f"{SPEC_JSON} differs from what this script produces from "
                    f"{EVIDENCE_JSON} — re-run scripts/build-cc-env-spec.py")
        if problems:
            sys.exit("cc-env-spec check FAILED:\n  " + "\n  ".join(problems))
        counts = {}
        for v in spec["vars"]:
            counts[v["control"]] = counts.get(v["control"], 0) + 1
        print(f"cc-env-spec OK: {spec['documented_count']} documented, "
              f"controls={counts}, vectors={len(json.load(open(VECTORS_JSON))['vectors'])}")
        return

    if vector_problems:
        sys.exit("Golden vectors do not match the spec:\n  "
                 + "\n  ".join(vector_problems))

    os.makedirs(DATA_DIR, exist_ok=True)
    with open(SPEC_JSON, "wb") as f:
        f.write(payload)
    counts = {}
    for v in spec["vars"]:
        counts[v["control"]] = counts.get(v["control"], 0) + 1
    print(f"spec      : {SPEC_JSON} ({len(payload)} bytes)")
    print(f"documented: {spec['documented_count']}  controls={counts}")
    print(f"crosscheck: {len(spec['documented_not_in_build'])} not-in-build, "
          f"{len(spec['undocumented_in_build'])} undocumented-in-build "
          f"(binary {spec['binary_crosscheck_version']})")

    if args.html:
        write_html(spec)


def write_html(spec: dict) -> None:
    """Authoring aid only — stays in dev-docs, never a build input."""
    tmpl_path = os.path.join(DEV_DOCS, "env-tools", "preview-template.html")
    if not os.path.exists(tmpl_path):
        print(f"warn: no preview template at {tmpl_path}; skipping HTML",
              file=sys.stderr)
        return
    with open(tmpl_path, encoding="utf-8") as f:
        tmpl = f.read()
    counts = {"total": spec["documented_count"],
              "missing": len(spec["documented_not_in_build"]),
              "undocumented": len(spec["undocumented_in_build"])}
    for kind in ("toggle", "enum", "number", "text"):
        counts[kind] = sum(1 for v in spec["vars"] if v["control"] == kind)
    meta = {
        "version": spec["binary_crosscheck_version"],
        "counts": counts,
        "categories": [{"k": k, "label": lbl} for k, lbl in CATEGORIES],
    }
    html = (tmpl.replace("__DATA__", json.dumps(spec["vars"], ensure_ascii=False))
                .replace("__META__", json.dumps(meta, ensure_ascii=False))
                .replace("__VERSION__", spec["binary_crosscheck_version"]))
    os.makedirs(os.path.dirname(OUT_HTML), exist_ok=True)
    with open(OUT_HTML, "w", encoding="utf-8") as f:
        f.write(html)
    print(f"html      : {OUT_HTML}")


if __name__ == "__main__":
    main()
