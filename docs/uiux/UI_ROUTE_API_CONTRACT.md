# UI Route and API Contract Baseline

This document specifies the exact baseline route contracts, JSON request/response shapes, error scenarios, and transport constraints for the Toolcase Gateway Odoo-inspired management console.

## 1. System UI and Asset Routes

| Method | Path | Request Body | Response Content-Type | Status | Description |
|---|---|---|---|---|---|
| `GET`, `HEAD` | `/dashboard` | None | `text/html; charset=utf-8` | 200 | Serves single-binary embedded dashboard HTML bootstrapped with security headers. Legacy `/` and `/gateway-config/dashboard` were removed and return 404. |
| `GET`, `HEAD` | `/gateway-ui/dashboard.css` | None | `text/css; charset=utf-8` | 200 / 404 | Allowlisted self-hosted styling asset. |
| `GET`, `HEAD` | `/gateway-ui/dashboard.js` | None | `application/javascript; charset=utf-8` | 200 / 404 | Allowlisted self-hosted client router and component controller. |
| `GET` | `/image-template` | None | `text/html; charset=utf-8` | 200 | Direct URL to dedicated Muse Image Generation Studio. |
| `GET` | `/video-template` | None | `text/html; charset=utf-8` | 200 | Direct URL to dedicated Muse Video Generation Studio. |
| `GET` | `/muse-config` | None | `text/html; charset=utf-8` | 200 | Direct URL to Muse HAR Import and Credentials Configuration. |

## 2. Dashboard REST Endpoints

### 2.1 Overview (`GET /api/v1/dashboard/overview`)
- **Headers:** None required. Cross-origin permitted via `Access-Control-Allow-Origin: *`.
- **Response Shape (200 OK):**
```json
{
  "config": {
    "target_host": "127.0.0.1",
    "target_port": 20128,
    "default_model": "gpt-5.6-sol",
    "io_timeout_secs": 120,
    "fallbacks": ["fail-try"]
  },
  "providers": {
    "omniroute": { "target_host": "127.0.0.1", "target_port": 20128, "default_model": "gpt-5.6-sol" },
    "jev": { "total_accounts": 1, "accounts": [{ "label": "account-1", "masked_key": "sk-dyn...-999" }] },
    "openrouter_jev": { "enabled": true, "credit_cooldown_secs": 300, "total_accounts": 0, "accounts": [] },
    "museai": { "configured": false, "approval": { "mode": "off", "allowed_destinations": [], "allowed_types": [] } }
  },
  "modules": [
    { "id": "omniroute", "name": "OmniRoute LLM Proxy", "namespace": "/ominroute", "kind": "builtin", "status": "enabled", "capabilities": ["chat_completions", "models"] },
    { "id": "muse", "name": "Muse AI Subsystem", "namespace": "/muse-ai", "kind": "builtin", "status": "enabled", "capabilities": ["chat", "models", "video", "image"] },
    { "id": "jev", "name": "Jev Decision Provider", "namespace": "/jev", "kind": "builtin", "status": "enabled", "capabilities": ["decisions", "models"] }
  ],
  "catalog": [...],
  "mcp": { "status": "enabled", "transports": ["stdio", "sse", "streamable_http"], "tools": [...] }
}
```

### 2.2 Module State (`POST /api/v1/dashboard/modules/state`)
- **Request Shape:**
```json
{ "id": "omniroute", "enabled": true }
```
- **Responses:**
  - `200 OK`: `{"ok": true, "id": "omniroute", "enabled": true}`
  - `400 Bad Request`: invalid JSON or unknown module ID
  - `403 Forbidden`: Cross-origin mutation without matching Host

### 2.3 Module Install (`POST /api/v1/dashboard/modules/install`)
- **Request Shape (Catalog Template):**
```json
{ "id": "cohere-provider" }
```
- **Request Shape (Custom Archive/Repository):**
```json
{
  "source": "https://github.com/example/custom-tool",
  "zip_base64": null,
  "id": "custom-tool",
  "name": "Custom Tool",
  "namespace": "/custom/tool",
  "entrypoint": "run.sh",
  "capabilities": ["chat_completions"]
}
```
- **Responses:**
  - `200 OK`: `{"ok": true, "module": {...}}`
  - `400 Bad Request`: Namespace collision or missing required manifest properties
  - `409 Conflict`: Path collision with reserved system routes

### 2.4 Module Removal (`POST /api/v1/dashboard/modules/remove`)
- **Request Shape:**
```json
{ "id": "custom-tool" }
```
- **Responses:**
  - `200 OK`: `{"ok": true, "message": "Module removed"}`
  - `400 Bad Request`: Built-in modules cannot be uninstalled

### 2.5 Provider Configuration Mutations
- `POST /api/v1/dashboard/config` -> Gateway upstream proxy configuration
- `POST /api/v1/dashboard/muse-config` -> Muse AI subsystem and Sentinel approval policies
- `POST /api/v1/dashboard/jev-config` -> Jev endpoint and OpenRouter failover parameters
- `POST /api/v1/dashboard/accounts` -> Account pool credential ingest (Jev API keys, OpenRouter keys, session cookies)
