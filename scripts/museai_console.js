(async () => {
  if (location.hostname !== "muse.ai" && !location.hostname.endsWith(".muse.ai")) {
    throw new Error("Run this script on https://muse.ai");
  }

  const values = new Map([["GW_MUSEAI_BASE_URL", location.origin]]);
  const originalFetch = window.fetch;
  const OriginalWebSocket = window.WebSocket;
  const seen = new WeakSet();

  function scan(value) {
    if (!value || typeof value !== "object" || seen.has(value)) return;
    seen.add(value);

    for (const [key, child] of Object.entries(value)) {
      const normalized = key.toLowerCase();
      if (typeof child === "string" && child) {
        if (normalized === "access_token" || normalized === "token") {
          if (!values.has("GW_MUSEAI_ACCESS_TOKEN")) {
            values.set("GW_MUSEAI_ACCESS_TOKEN", child);
          }
        } else if (["ws_token", "websocket_token", "notary_token"].includes(normalized)) {
          values.set("GW_MUSEAI_NOTARY_TOKEN", child);
        } else if (
          ["endpoint_url", "websocket_url", "websocket_endpoint", "ws_url"].includes(normalized) &&
          /^wss?:\/\//.test(child)
        ) {
          values.set("GW_MUSEAI_WS_URL", child);
        }
      }
      scan(child);
    }
  }

  function scanStorage(storage) {
    for (let index = 0; index < storage.length; index += 1) {
      const key = storage.key(index);
      const value = key ? storage.getItem(key) : null;
      if (!key || !value) continue;
      try {
        scan({ [key]: JSON.parse(value) });
      } catch {
        scan({ [key]: value });
      }
    }
  }

  function envValue(value) {
    return /[\s#;"']/.test(value) ? JSON.stringify(value) : value;
  }

  async function copyConfig() {
    const text = [...values]
      .map(([key, value]) => `${key}=${envValue(value)}`)
      .join("\n");

    try {
      await navigator.clipboard.writeText(`${text}\n`);
    } catch {
      const textarea = document.createElement("textarea");
      textarea.value = `${text}\n`;
      textarea.style.position = "fixed";
      textarea.style.opacity = "0";
      document.body.appendChild(textarea);
      textarea.select();
      document.execCommand("copy");
      textarea.remove();
    }

    const keys = [...values.keys()];
    console.info(`Copied ${keys.length} config keys: ${keys.join(", ")}`);
    return keys;
  }

  window.fetch = async function museAiConfigFetch(...args) {
    const response = await originalFetch.apply(this, args);
    if (response.url.startsWith(`${location.origin}/api/`)) {
      response.clone().json().then(scan).catch(() => {});
    }
    return response;
  };

  function CapturingWebSocket(url, protocols) {
    if (/^wss?:\/\//.test(String(url))) {
      values.set("GW_MUSEAI_WS_URL", String(url));
    }
    return protocols === undefined
      ? new OriginalWebSocket(url)
      : new OriginalWebSocket(url, protocols);
  }
  CapturingWebSocket.prototype = OriginalWebSocket.prototype;
  for (const key of ["CONNECTING", "OPEN", "CLOSING", "CLOSED"]) {
    Object.defineProperty(CapturingWebSocket, key, { value: OriginalWebSocket[key] });
  }
  window.WebSocket = CapturingWebSocket;

  scanStorage(localStorage);
  scanStorage(sessionStorage);

  try {
    const response = await originalFetch("/api/session", {
      credentials: "include",
      headers: { Accept: "application/json" },
    });
    if (response.ok) scan(await response.json());
    else console.warn(`Muse.ai session request returned HTTP ${response.status}`);
  } catch (error) {
    console.warn(`Muse.ai session request failed: ${error.message}`);
  }

  if (document.cookie) {
    values.set("GW_MUSEAI_COOKIE", document.cookie);
  }

  window.museAiConfig = Object.freeze({
    copy: copyConfig,
    keys: () => [...values.keys()],
    stop() {
      window.fetch = originalFetch;
      window.WebSocket = OriginalWebSocket;
      delete window.museAiConfig;
      console.info("Muse.ai capture stopped");
    },
  });

  await copyConfig();
  console.info("Capture active. Start/connect Muse.ai, then run: await museAiConfig.copy()");
  console.warn(
    "GW_MUSEAI_COOKIE can omit HttpOnly cookies. If requests fail, copy full Cookie request header from DevTools Network > /api/session > Headers."
  );
})();
