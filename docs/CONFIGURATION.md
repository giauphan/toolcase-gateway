# Configuration Reference

`toolcase-gateway` provides a dual-layer configuration model:
1. **Startup Bootstrapping**: Initial settings are loaded from environment variables and an optional `.env` file (configured via `GW_ENV_FILE`).
2. **Standalone Persistent Storage (`gateway_config.json`)**: Settings managed through the Dashboard UI (`/gateway-config/dashboard`) or management APIs are stored in a standalone, crash-safe JSON file (configured via `GW_CONFIG_FILE`), leaving `.env` untouched.

---

## 1. Configuration Precedence

When the gateway initializes and handles requests, configuration resolves in the following strict order of precedence:

```
[Built-in Defaults]
        ↓ overridden by
[Environment Variables & .env file]
        ↓ overridden by
[Persistent JSON Storage (gateway_config.json)]
        ↓ overridden by
[Live Runtime Dashboard API Mutations]
```

1. **Built-in Defaults**: Hardcoded safe fallbacks in `Config::default()`.
2. **Environment & `.env`**: Read on initial boot.
3. **Persistent Config Store (`gateway_config.json`)**: If the JSON file exists on disk, its defined keys overlay the boot configuration.
4. **Live In-Memory State (`ConfigStore`)**: Changes submitted via `/api/v1/dashboard/config`, `/api/v1/dashboard/muse-config`, or `/api/v1/dashboard/accounts` take effect immediately without process restart and are committed atomically to `gateway_config.json`.

---

## 2. Environment Variables Reference

| Variable | Default | Type | Subsystem | Description |
| :--- | :--- | :--- | :--- | :--- |
| `GW_CONFIG_FILE` | `gateway_config.json` | path | Core | Path to the dedicated persistent JSON configuration file. |
| `GW_ENV_FILE` | `.env` | path | Core | Path to the environment bootstrap file. |
| `GW_LISTEN_HOST` | `127.0.0.1` | host | Core | Bind IP address. Non-loopback addresses emit a security warning. |
| `GW_LISTEN_PORT` | `20129` | u16 | Core | TCP listen port. |
| `GW_TARGET_HOST` | `127.0.0.1` | host | OmniRoute | Upstream OpenAI-compatible host to proxy requests to. |
| `GW_TARGET_PORT` | `20128` | u16 | OmniRoute | Upstream OpenAI-compatible target port. |
| `GW_DEFAULT_MODEL` | *(empty)* | string | OmniRoute | Default model substituted when client request specifies an unknown model. |
| `GW_FALLBACK_MODELS` | `fail-try` | csv | OmniRoute | Comma-separated candidate models attempted in round-robin on retryable errors. |
| `GW_MAX_CONNECTIONS` | `256` | usize | Runtime | Concurrent TCP connection cap. Excess connections receive `503`. |
| `GW_IO_TIMEOUT_SECS` | `120` | u64 | Runtime | Socket read and write timeout in seconds (`0` disables timeout). |
| `GW_RETRY_BASE_DELAY_MS` | `100` | u64 | OmniRoute | Base backoff delay in milliseconds before candidate failover. |
| `GW_MAX_RETRY_DELAY_MS` | `5000` | u64 | OmniRoute | Maximum backoff delay cap in milliseconds. |
| `GW_JEV_API_KEYS` | *(empty)* | csv | Jev AI | Comma-separated list of Jev API keys (`sk-glm...`) for account pool rotation. Entries can include optional label prefixes (`label:key` or `label=key`). |
| `GW_JEV_BASE_URL` | `https://jev-ai.org` | URL | Jev AI | Target base URL for the direct Jev Decision API. |
| `GW_JEV_OPENROUTER_BASE_URL` | `https://openrouter.ai` | URL | Jev AI | Upstream base URL for OpenRouter Decisions API (`/api/alpha/decisions`). |
| `GW_JEV_OPENROUTER_API_KEY` | *(empty)* | string | Jev AI | OpenRouter API key (`sk-or-...`) used for Bearer token authentication to OpenRouter decisions (`/api/alpha/decisions`). |
| `GW_JEV_OPENROUTER_API_KEYS` | *(empty)* | csv | Jev AI | Comma-separated OpenRouter API keys (`sk-or-...`) with optional label prefixes for pool rotation. |
| `GW_JEV_OPENROUTER_COOKIE` | *(empty)* | string | Jev AI | Browser session cookie used for OpenRouter decision endpoint authentication when no API key is set. |
| `GW_JEV_OPENROUTER_COOKIES` | *(empty)* | string | Jev AI | Line-separated OpenRouter browser session cookies with optional label prefixes for pool rotation. |
| `GW_JEV_OPENROUTER_MODEL` | `typesafe/jev-1.13` | string | Jev AI | Default decision model identifier routed to OpenRouter. |
| `GW_JEV_OPENROUTER_FALLBACK_MODELS` | `baseten/mammoth-1:free,nace-ai/drex-max` | csv | Jev AI | Comma-separated fallback decision models attempted and round-robined on OpenRouter in Step 3 if primary decision model fails. |
| `GW_JEV_OPENROUTER_ENABLED` | `true` | bool | Jev AI | Flag indicating whether failover to OpenRouter browser provider is enabled. |
| `GW_JEV_CREDIT_COOLDOWN_SECS` | `300` | u64 | Jev AI | Duration in seconds a direct account returning HTTP 402 is demoted before retry. |
| `GW_JEV_ENABLED` | `false` | bool | Jev AI | Flag indicating whether Jev provider routing is active. |
| `GW_JEV_SHADOW_MODE` | `true` | bool | Jev AI | Enables non-blocking shadow routing evaluation for telemetry. |
| `GW_MUSEAI_BASE_URL` | `https://muse.ai` | URL | Muse AI | Base URL for Muse REST APIs and artifact downloads. |
| `GW_MUSEAI_WS_URL` | `wss://hatch.metaaivm.com/v1/noise` | URL | Muse AI | WebSocket endpoint for Noise XX Confidential VM sessions. |
| `GW_MUSEAI_ACCESS_TOKEN` | *(empty)* | string | Muse AI | Bearer access token for Muse CVM authentication. |
| `GW_MUSEAI_NOTARY_TOKEN` | *(empty)* | string | Muse AI | Attestation notary token used during Noise XX handshake. |
| `GW_MUSEAI_VM_ID` | *(empty)* | string | Muse AI | Identifier of the target Confidential VM instance. |
| `GW_MUSEAI_COOKIE` | *(empty)* | string | Muse AI | Session cookie (`hatch_sess=...`) for REST and WebSocket bootstrap. |
| `GW_MUSEAI_AUTO_CLEANUP_THREADS` | `true` | bool | Muse AI | Automatically clean up background worker threads after retention expiry. |
| `GW_MUSEAI_THREAD_RETENTION_SECS` | `172800` | u64 | Muse AI | Duration in seconds before inactive worker sessions, video jobs, and media are reclaimed (default 2 days: 172800s). |
| `MUSE_LIVE_VIDEO_PROMPT` | *(built-in prompt)* | string | Muse Video | Default prompt template used when generating videos without explicit prompt. |
| `GW_MUSE_APPROVAL_MODE` | `off` | enum | Muse Sentinel | Approval mode: `off` (fail-closed), `observe` (log only), or `on` (scoped auto-approval). |
| `GW_MUSE_APPROVAL_ALLOWED_DESTINATIONS` | *(empty)* | csv | Muse Sentinel | Comma-separated domain patterns permitted for auto-approval (e.g. `muse.ai`). |
| `GW_MUSE_APPROVAL_ALLOWED_TYPES` | `network` | csv | Muse Sentinel | Decision categories eligible for headless approval (e.g. `network`). |
| `GW_MUSE_APPROVAL_PREFER_ALLOW_ONCE` | `true` | bool | Muse Sentinel | Default to single-decision permission grant. |
| `GW_MUSE_APPROVAL_ALLOW_ALWAYS` | `false` | bool | Muse Sentinel | Allow persistent rule creation for pre-scoped destinations. |
| `GW_MUSE_APPROVAL_MAX_DECISIONS` | `3` | usize | Muse Sentinel | Maximum number of automated decisions allowed per video generation job. |
| `GW_MUSE_APPROVAL_TIMEOUT_SECS` | `30` | u64 | Muse Sentinel | Seconds to wait for an auto-approval decision before timing out. |
| `GW_PRISM_BASE_URL` | `https://prism.openai.com` | URL | Prism | Upstream base URL for dedicated Prism completions. |
| `GW_PRISM_PROJECT_ID` | *(built-in default)* | string | Prism | Default project ID for Prism sessions. |
| `GW_PRISM_DEFAULT_MODEL` | `gpt-5.6-sol` | string | Prism | Default reasoning model for Prism requests. |
| `GW_LOG_DIR` | `logs` | path | Logging | Directory where runtime log files are automatically created and stored. |
| `GW_LOG_FILE` | `gateway.log` | string | Logging | Primary log filename within `GW_LOG_DIR`. |
| `GW_LOG_ENABLED` | `true` | bool | Logging | Persist log entries to disk in `GW_LOG_DIR`. |
| `GW_LOG_DAILY` | `true` | bool | Logging | Enable daily rotating log files (`gateway-YYYY-MM-DD.log`, `error-YYYY-MM-DD.log`, `access-YYYY-MM-DD.log`). |
| `GW_LOG_MAX_SIZE_MB` | `10` | u64 | Logging | Maximum size in megabytes before an active log file is rotated and zipped into a `.zip` archive. Defaults to 10 MB. |
| `GW_LOG_ZIP_ENABLED` | `true` | bool | Logging | Automatically compress rotated log files exceeding the size threshold into standard PKZIP archives. |
| `GW_LOG_MAX_SIZE_BYTES` | *(optional)* | u64 | Logging | Exact byte threshold override for log file rotation and zip compression. |
| `GW_LOG_RETENTION_DAYS` | `2` | u64 | Logging | Days to retain rotated daily log files and archived `.zip` files before automatic daemon cleanup (defaults to 2 days). |
| `GW_LOG_CONSOLE` | `true` | bool | Logging | Mirror formatted log entries to standard error (`stderr`). |

### Logging Streams and Format

The gateway writes structured log streams into `GW_LOG_DIR` (`logs/`):
- **`access.log`** (`access-YYYY-MM-DD.log`): Records every handled HTTP request including client IP, HTTP method, sanitized path, final HTTP response status code, and elapsed duration in milliseconds:
  ```text
  [2026-10-24T04:19:34Z] [ACCESS] 127.0.0.1 "POST /v1/responses" 200 45ms
  ```
- **`gateway.log`** (`gateway-YYYY-MM-DD.log`): Comprehensive runtime log combining subsystem events (`gateway_proxy`, `museai_chat`, `museai_video`, `jev_ai`) and mirrored access events:
  ```text
  [2026-10-24T04:19:34Z] [INFO] [gateway_proxy] Routing request /v1/responses to model 'gpt-4o' (candidates: gpt-4o)
  [2026-10-24T04:19:34Z] [INFO] [gateway_proxy] Forwarding request /v1/responses to upstream 127.0.0.1:20128 with model 'gpt-4o' (attempt 1/1)
  [2026-10-24T04:19:34Z] [INFO] [gateway_proxy] Upstream model 'gpt-4o' responded with HTTP 200 OK for path /v1/responses
  [2026-10-24T04:19:34Z] [ACCESS] 127.0.0.1 "POST /v1/responses" 200 45ms
  ```
- **`error.log`** (`error-YYYY-MM-DD.log`): Error stream isolating warnings, upstream failovers, network failures, and 5xx conditions.

---

## 3. Standalone Persistent JSON Storage (`gateway_config.json`)

To prevent process race conditions, credential theft, and corruption of the development `.env` file, the gateway uses a dedicated JSON file (`gateway_config.json`) for dashboard mutations.

### JSON Schema & Example
```json
{
  "target_host": "127.0.0.1",
  "target_port": 20128,
  "fallbacks": [
    "fail-try",
    "gpt-4o"
  ],
  "default_model": "gpt-5.6-sol",
  "io_timeout_secs": 120,
  "jev_api_keys": [
    "sk-glm5-00bc9802bf5fd3545cbd00745429569fa4e871adb0c1d74b",
    "sk-glm5-secondary-backup-token-example"
  ],
  "jev_base_url": "https://jev-ai.org",
  "museai_base_url": "https://muse.ai",
  "museai_ws_url": "wss://hatch.metaaivm.com/v1/noise",
  "museai_vm_id": "vm-29402941",
  "museai_access_token": "sk-secret-bearer-token",
  "museai_notary_token": "notary-secret-token",
  "museai_cookie": "hatch_sess=...; datr=...",
  "default_video_prompt": "A colorful butterfly landing gently on a blooming sunflower, macro video",
  "muse_approval_mode": "on",
  "muse_approval_allowed_types": [
    "network"
  ],
  "muse_approval_allowed_destinations": [
    "muse.ai",
    "api.muse.ai"
  ],
  "muse_approval_prefer_allow_once": true,
  "muse_approval_allow_always": false,
  "muse_approval_max_decisions": 5,
  "muse_approval_timeout_secs": 45
}
```

### Atomic File Write Guarantees
1. Changes are written to a temporary sibling file (`.gateway_config.json~tmp`).
2. On Unix platforms, permissions are strictly restricted to `0600` (`-rw-------`), readable only by the process user.
3. The temporary file is atomically swapped over the destination via `std::fs::rename`. This eliminates partial writes or corrupted JSON if the process is terminated mid-write.
4. The `.env` file remains untouched.

---

## 4. Compile-Time Limits

These limits are defined in `src/http.rs` and `src/runtime/` to prevent memory exhaustion and request smuggling:

| Constant | Value | Purpose |
| :--- | :--- | :--- |
| `MAX_HEAD_BYTES` | 64 KB | Maximum allowed byte length for HTTP request and response headers. |
| `MAX_REQUEST_BYTES` | 64 MB | Maximum allowed byte length for HTTP request body payloads. |
| `MAX_REWRITE_WINDOW` | 1 MB | Buffer threshold forcing flush if no line breaks are encountered in SSE streams. |
| `DEFAULT_MAX_IN_FLIGHT_BYTES` | 64 MB | Global gateway admission budget for concurrent payload buffering. |
| `RETRYABLE` | `401, 402, 403, 408, 429, 499, 500, 502, 503, 504, 524` | Upstream HTTP status codes that trigger model failover. |

---

## 5. Dashboard Configuration Guide

Access the interactive dashboard at:
```
http://127.0.0.1:20129/gateway-config/dashboard
```

- **Gateway Configuration**: Adjust upstream OmniRoute host, port, default model, I/O timeout, and fallback model chains.
- **Muse AI Subsystem**: Configure REST and WebSocket endpoints, VM ID, bearer tokens, session cookies, default video prompt, and Sentinel auto-approval rules. Leaving credential fields blank preserves currently active tokens.
- **Provider Account Pool**: View active Jev AI accounts with descriptive labels and masked keys. Paste new keys (comma- or newline-separated) with optional labels (using the dedicated Label input or inline `label:key` / `label=key` syntax) to dynamically expand the round-robin rotation pool without restarting the server. Custom labels persist to standalone JSON configuration.
