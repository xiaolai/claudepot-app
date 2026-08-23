// Connectivity probe for the Claudepot remote surface.
//
// Its whole job is to answer questions that cannot be answered from a
// development machine: does this browser treat a privately-trusted
// certificate on a .internal name as a SECURE CONTEXT, will it install
// as a PWA, and is a platform authenticator (Face ID / Touch ID)
// available for a future passkey login.
//
// Everything it reports is a browser capability. It sends nothing
// anywhere and reads nothing about the host.

const $ = (id) => document.getElementById(id);

function row(k, ok, detail) {
  const d = document.createElement("div");
  d.className = "row";
  const kk = document.createElement("span");
  kk.className = "k";
  kk.textContent = k;
  const vv = document.createElement("span");
  vv.className = "v " + (ok === null ? "" : ok ? "yes" : "no");
  vv.textContent = detail ?? (ok === null ? "?" : ok ? "yes" : "no");
  d.append(kk, vv);
  return d;
}

async function probe() {
  const env = $("env");
  const secure = window.isSecureContext === true;
  env.append(row("secure context", secure));
  env.append(row("protocol", location.protocol === "https:", location.protocol));
  env.append(
    row(
      "installed (standalone)",
      null,
      matchMedia("(display-mode: standalone)").matches ||
        window.navigator.standalone === true
        ? "yes"
        : "no — use Share ▸ Add to Home Screen",
    ),
  );
  env.append(row("service worker API", "serviceWorker" in navigator));
  env.append(row("crypto.subtle", !!(window.crypto && window.crypto.subtle)));

  // The question that decides whether passkeys are worth building.
  const hasPk = typeof window.PublicKeyCredential !== "undefined";
  env.append(row("WebAuthn API", hasPk));
  if (hasPk && PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable) {
    try {
      const ok = await PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable();
      env.append(row("platform authenticator", ok, ok ? "yes (Face ID / Touch ID)" : "no"));
    } catch (e) {
      env.append(row("platform authenticator", false, "error"));
    }
  }

  // The row above answers "does this DEVICE have an authenticator". It
  // says nothing about whether THIS ORIGIN may use one, and the two come
  // apart precisely here: WebAuthn's RP ID must be a valid domain, and
  // an IP-address origin has none. Reporting only the capability would
  // be a green light measuring the wrong thing.
  //
  // So: derive the RP ID the way a browser does, and say plainly when
  // the origin disqualifies itself before any prompt appears.
  const host = location.hostname;
  const isIp =
    /^\d{1,3}(\.\d{1,3}){3}$/.test(host) || host.includes(":") || host.startsWith("[");
  env.append(
    row(
      "origin usable for passkeys",
      !isIp,
      isIp
        ? `no — ${host} is an IP, and WebAuthn needs a domain`
        : `yes — rp.id would be ${host}`,
    ),
  );
  if (isIp) {
    $("envnote2").textContent =
      "Passkeys need a hostname, not an IP. This certificate already covers " +
      "the .local and MagicDNS names, so reaching the same server by name " +
      "is enough — no re-mint required.";
  }

  // Registering is the real test — the API can exist and still refuse
  // outside a secure context.
  if ("serviceWorker" in navigator) {
    try {
      const reg = await navigator.serviceWorker.register("/sw.js", { scope: "/" });
      env.append(row("service worker registered", true, reg.scope));
    } catch (e) {
      env.append(row("service worker registered", false, String(e.message || e).slice(0, 60)));
    }
  }

  $("envnote").textContent = secure
    ? "Secure context confirmed — a PWA and web push are possible here."
    : "NOT a secure context. No service worker, no install, no push. " +
      "If the padlock is missing, the CA profile is installed but full " +
      "trust is probably not enabled (Settings ▸ General ▸ About ▸ " +
      "Certificate Trust Settings).";
}

async function api(path, opts = {}) {
  const res = await fetch(path, {
    ...opts,
    headers: {
      "content-type": "application/json",
      ...(opts.headers || {}),
    },
  });
  let body = null;
  try {
    body = await res.json();
  } catch {}
  return { status: res.status, body };
}

async function showSession(token) {
  const { status, body } = await api("/api/me", {
    headers: { authorization: "Bearer " + token },
  });
  if (status !== 200) {
    localStorage.removeItem("claudepot.token");
    return false;
  }
  $("loginbox").hidden = true;
  $("session").hidden = false;
  $("who").append(row("device", null, body.device ?? "?"));
  $("who").append(row("expires", null, (body.expires_at || "never").slice(0, 16)));
  return true;
}

$("f").addEventListener("submit", async (e) => {
  e.preventDefault();
  const msg = $("msg");
  const go = $("go");
  go.disabled = true;
  msg.textContent = "…";
  const { status, body } = await api("/api/login", {
    method: "POST",
    body: JSON.stringify({
      password: $("pw").value,
      totp: $("totp").value || undefined,
      device_label: navigator.userAgent.includes("iPhone") ? "iPhone" : "browser",
    }),
  });
  go.disabled = false;
  $("pw").value = "";

  if (status === 200 && body.token) {
    localStorage.setItem("claudepot.token", body.token);
    msg.textContent = "";
    await showSession(body.token);
    return;
  }
  // Deliberately echoes the server's own wording rather than inventing
  // friendlier copy: the server is careful not to say which half of a
  // credential was wrong, and a helpful client would undo that.
  const map = {
    invalid: "Wrong password or code.",
    totp_required: "Enter the code from your authenticator.",
    throttled: `Too many attempts. Wait ${body.retry_after_secs ?? "a moment"}s.`,
    not_configured: "No password is set on this Claudepot yet.",
    bad_host: "This address is refused by the server.",
    bad_origin: "Cross-site request refused.",
  };
  msg.className = "no";
  msg.textContent = map[body?.error] || `Failed (${status}).`;
});

(async () => {
  await probe();
  const saved = localStorage.getItem("claudepot.token");
  if (saved) await showSession(saved);
})();
