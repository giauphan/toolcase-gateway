# Platform Modules & Plugin Architecture

Toolcase Gateway features an extensible, self-contained module and plugin architecture modeled after the **WordPress plugin** and **Odoo addon** paradigms.

External developers and operators can author, package, install, and run provider modules without needing the gateway source code or recompiling the binary.

---

## 1. Architectural Concept

Just like WordPress plugins (`wp-content/plugins/<plugin-slug>/`) and Odoo modules (`addons/<module_name>/` with `__manifest__.py`):

1. **Self-Contained Bundles**: Every module is an independent directory under `$GW_STATE_DIR/modules/<module-id>/` (or `./modules/<module-id>/`). It bundles its own manifest (`module.json`, `manifest.json`, or `__manifest__.json`), executables, scripts, configuration, and assets.
2. **Auto-Discovery**: The gateway automatically scans the modules directory on boot. Any valid folder containing a manifest is discovered and registered.
3. **Dedicated URL Namespace**: Each module owns an isolated public URL prefix (e.g. `/custom/my-provider/...`), completely separate from core gateway routes (`/omniroute/...`, `/muse-ai/...`, `/jev/...`).
4. **Out-of-Process Isolation**: Modules run in isolated child processes (`stdio-stream`) or loopback HTTP services (`local-http`). If a module panics or crashes, the gateway isolates the failure, returns a local `502 Bad Gateway`, and remains healthy.
5. **Zero Secret Leakage**: Subprocess environments are sanitized. Gateway master keys, cookies, and tokens are never passed to external modules.

```
toolcase-gateway/
├── gateway_config.json
├── logs/
└── .gw_state/
    └── modules/
        ├── custom-cohere/            <-- Self-contained module
        │   ├── module.json           <-- Declarative manifest
        │   ├── run.sh                <-- Executable entrypoint
        │   └── adapter.py
        └── custom-anthropic/
            ├── __manifest__.json     <-- Odoo-style manifest
            └── index.js
```

---

## 2. Module Lifecycle

A platform module passes through distinct lifecycle states:

```
[GitHub Repo / ZIP / Folder]
             │
             ▼ (Install / Convert)
       [ Installed ] (Files unpacked into modules/<id>/)
             │
      ┌──────┴──────┐
      ▼             ▼
  [ Enabled ]   [ Disabled ]
  (Serves       (Locally fails
   requests      closed 503;
   at namespace) files preserved)
      │             │
      └──────┬──────┘
             ▼ (Uninstall)
       [ Uninstalled ] (Files purged from disk)
```

- **Installed**: Extracted into its dedicated directory, manifest validated, and registered in the platform registry.
- **Enabled (Active)**: Mounted at its declared route namespace. Requests matching its prefix are forwarded to its entrypoint.
- **Disabled (Inactive)**: Retained on disk with settings intact. Requests to its namespace fail closed immediately (`503 Service Unavailable`).
- **Uninstalled**: Unregistered from the gateway and permanently removed from disk.

---

## 3. Manifest Specification

A module declares its identity and capabilities in `module.json` (also accepted: `manifest.json` or `__manifest__.json`):

```json
{
  "id": "cohere-adapter",
  "name": "Cohere Provider Adapter",
  "version": "1.0.0",
  "namespace": "/custom/cohere",
  "entrypoint": "run.sh",
  "protocol": "stdio-stream",
  "capabilities": [
    "chat_completions",
    "models"
  ],
  "config_schema": {
    "api_key": { "type": "string", "required": true }
  },
  "permissions": [
    "network"
  ]
}
```

### Manifest Fields

| Field | Type | Description |
|---|---|---|
| `id` | `string` | Unique identifier (alphanumeric, `-`, `_`). Must not conflict with built-in modules (`omniroute`, `muse`, `jev`). |
| `name` | `string` | Human-readable display name shown in dashboard and MCP tools. |
| `version` | `string` | SemVer string (e.g. `"1.0.0"`). |
| `namespace` | `string` | Canonical URL prefix (e.g. `"/custom/cohere"`). Must start with `/` and not shadow system routes (`/api`, `/dashboard`, `/mcp`, etc.). |
| `entrypoint` | `string` | Path to executable or script. If relative (e.g. `"run.sh"` or `"./main.py"`), resolved relative to module folder. |
| `protocol` | `string` | Communication protocol: `"stdio-stream"` (default) or `"local-http"`. |
| `capabilities` | `string[]` | Advertised capabilities (e.g. `["chat_completions", "models"]`). |
| `config_schema`| `object` | Optional JSON schema for module configuration options. |
| `group` | `object` | Optional Module Group configuration bundling multiple sub-providers. |

### 3.1 Module Groups (Bundling Multiple Providers)

Instead of authoring, installing, and maintaining dozens of distinct single-purpose modules, Toolcase Gateway supports **Module Groups**. A single module package can declare a `group` containing multiple sub-providers, all mounted under the parent module's URL namespace:

```json
{
  "id": "office-ai-suite",
  "name": "Office AI Multi-Provider Suite",
  "version": "1.0.0",
  "namespace": "/custom/office-ai",
  "entrypoint": "run.sh",
  "protocol": "stdio-stream",
  "capabilities": ["chat_completions", "embeddings", "vision"],
  "group": {
    "enabled": true,
    "providers": [
      {
        "id": "chat-fast",
        "name": "Fast LLM Chat",
        "subpath": "/chat",
        "capabilities": ["chat_completions"]
      },
      {
        "id": "embed-dense",
        "name": "Dense Text Embeddings",
        "subpath": "/embed",
        "capabilities": ["embeddings"]
      },
      {
        "id": "vision-ocr",
        "name": "Document Vision OCR",
        "subpath": "/vision",
        "capabilities": ["vision"]
      }
    ]
  }
}
```

#### How Module Group Routing Works
- Requests to `/custom/office-ai/chat/completions` automatically route to `run.sh` with environment variable `MODULE_SUB_PROVIDER_ID=chat-fast`.
- Requests to `/custom/office-ai/embed` route with `MODULE_SUB_PROVIDER_ID=embed-dense`.
- Disabling the module group (`office-ai-suite`) immediately disables all sub-providers with `503 Service Unavailable`.
- Sub-providers can also be toggled individually (`set_sub_provider_status`).

---

## 4. How to Install Modules

### Option A: From GitHub Repository

Provide the GitHub URL. The installer downloads the branch archive, verifies or generates the manifest, and extracts it:

```sh
curl -X POST http://127.0.0.1:20130/api/v1/dashboard/modules/install \
  -H "Content-Type: application/json" \
  -d '{
    "source": "https://github.com/my-org/my-gateway-module",
    "branch": "main"
  }'
```

### Option B: From ZIP Archive (URL or Upload)

1. **Via URL**:
   ```sh
   curl -X POST http://127.0.0.1:20130/api/v1/dashboard/modules/install \
     -H "Content-Type: application/json" \
     -d '{
       "source": "https://example.com/downloads/my-module.zip"
     }'
   ```

2. **Via Web Dashboard Upload**:
   - Navigate to `http://127.0.0.1:20130/dashboard#module-catalog`.
   - Open **Install Custom Module (GitHub Repo, ZIP Archive, or Upload)**.
   - Select your `.zip` archive or paste a GitHub link.
   - Click **Install Module**.

3. **Via Base64 Payload**:
   ```sh
   curl -X POST http://127.0.0.1:20130/api/v1/dashboard/modules/install \
     -H "Content-Type: application/json" \
     -d '{
       "zip_base64": "<base64-encoded-zip-data>",
       "id": "my-module"
     }'
   ```

### Option C: Direct Drop-in (Odoo / WordPress Style)

Copy or extract the module folder directly into `$GW_STATE_DIR/modules/<id>/` (default `.gw_state/modules/<id>/`):

```sh
mkdir -p .gw_state/modules/my-adapter
cp module.json run.sh adapter.py .gw_state/modules/my-adapter/
```

Restart or reload the gateway. The module is automatically discovered and registered!

### Option D: Via Model Context Protocol (MCP)

AI agents and IDEs can connect over Model Context Protocol using any of the 3 supported transports:
1. **stdio (Local)**: Local IDE connection where the IDE spawns the gateway process via `toolcase-gateway --mcp` (or `omniroute --mcp`).
2. **SSE (Remote)**: Remote Server-Sent Events connection over HTTP (`GET /mcp/sse`).
3. **Streamable HTTP (Remote)**: Remote modern bidirectional HTTP POST (`POST /mcp`).

Once connected, agents invoke the `gateway_install_personal_module` tool:

```json
{
  "name": "gateway_install_personal_module",
  "arguments": {
    "source": "https://github.com/my-org/my-gateway-module"
  }
}
```

---

## 5. Automated Conversion ("Any Things We Can Convert It")

If an external repository or zip archive does not contain a `module.json`, the gateway's built-in converter automatically synthesizes one:

1. **Identifier & Name**: Inferred from the repository name or directory name.
2. **Namespace**: Inferred as `/custom/<module-id>`.
3. **Entrypoint Detection**:
   - If `run.sh`, `start.sh`, or `adapter.sh` exists, it is marked executable and set as the entrypoint.
   - If `main.py` exists, a wrapper `run.sh` executing `python3 "$(dirname "$0")/main.py"` is generated.
   - If `index.js` exists, a wrapper `run.sh` executing `node "$(dirname "$0")/index.js"` is generated.
4. **Manifest Generation**: The synthesized `module.json` is written into the module directory, transforming the repository into a fully self-contained package.

---

## 6. How to Author a Custom Module

A ready-to-test reference module is included directly in the repository at [`examples/modules/echo-provider/`](../examples/modules/echo-provider/). You can use it as a starting template.

### Protocol: `stdio-stream`

The gateway spawns the module's entrypoint process and communicates over stdin/stdout using single-line JSON envelopes.

#### Inbound Payload (sent to module's stdin):

```json
{
  "method": "POST",
  "path": "/chat/completions",
  "headers": {
    "content-type": "application/json",
    "authorization": "Bearer ..."
  },
  "body": "{\"model\":\"custom\",\"messages\":[{\"role\":\"user\",\"content\":\"Hello\"}]}"
}
```

#### Outbound Payload (read from module's stdout):

```json
{
  "status": 200,
  "headers": {
    "content-type": "application/json"
  },
  "body": "{\"id\":\"chatcmpl-1\",\"choices\":[{\"message\":{\"role\":\"assistant\",\"content\":\"Hello from custom module!\"}}]}"
}
```

### Python Example (`adapter.py`)

```python
#!/usr/bin/env python3
import sys
import json

def handle_request(req):
    body = json.loads(req.get("body", "{}"))
    return {
        "status": 200,
        "headers": {"content-type": "application/json"},
        "body": json.dumps({
            "id": "custom-1",
            "object": "chat.completion",
            "choices": [{
                "message": {"role": "assistant", "content": "Echo: " + str(body.get("messages", []))}
            }]
        })
    }

for line in sys.stdin:
    if not line.strip():
        continue
    req = json.loads(line)
    res = handle_request(req)
    sys.stdout.write(json.dumps(res) + "\n")
    sys.stdout.flush()
```

---

## 7. Security & Sandboxing

1. **Zip Slip Prevention (CWE-22)**:
   - Archive extraction sanitizes every path component.
   - Any entry attempting to use `..`, drive letters, or absolute paths is rejected with `PathTraversal` error before writing.
2. **Path Containment**:
   - Module extraction ensures all files reside strictly inside the canonical target directory.
3. **Child Process Containment**:
   - Subprocesses execute with an isolated environment (`env_clear()`), retaining only an explicit allowlist of basic system variables (`PATH`, `HOME`, `LANG`, `TMPDIR`, `TZ`, `USER`) and non-secret contextual identifiers (`MODULE_ID`, `MODULE_NAMESPACE`, `MODULE_SUB_PROVIDER_ID`).
   - Standard timeouts (default 30 seconds) prevent runaway child processes.
4. **Namespace Collision Defense**:
   - The registry rejects any module attempting to claim or shadow built-in routes (`/omniroute`, `/muse-ai`, `/jev`) or system routes (`/api`, `/dashboard`, `/mcp`).

---

## 8. Managing Modules via Model Context Protocol (MCP)

AI agents and editor extensions can orchestrate the platform module lifecycle autonomously through native MCP tools:

| MCP Tool | Arguments | Purpose |
| :--- | :--- | :--- |
| `gateway_list_modules` | *(none)* | Returns JSON array of all registered modules, enabled status, kind, and namespaces |
| `gateway_get_module_info` | `{"module_id": "string"}` | Retrieves complete metadata, author info, capabilities, and sub-providers for a module |
| `gateway_set_module_state`| `{"module_id": "string", "enabled": bool}` | Dynamically activates or deactivates a module without server restart |
| `gateway_install_personal_module` | `{"source": "string", "source_type": "dir\|zip\|github"}` | Installs and registers an out-of-tree module from disk, ZIP, or remote GitHub repository |
| `gateway_remove_personal_module` | `{"module_id": "string"}` | Uninstalls and purges an installed module directory from disk |
| `gateway_get_system_metrics` | *(none)* | Returns live platform telemetry, memory usage, active connections, and module counters |

---

## 9. Built-in & External Catalog Modules

Toolcase Gateway bundles three built-in core modules (`omniroute`, `muse`, `jev`) and a System Module Catalog of 1-click installable external modules (such as `tiktok-studio` hosted at [`https://github.com/giauphan/TikTok-Creator-Studio-Adapter.git`](https://github.com/giauphan/TikTok-Creator-Studio-Adapter)):

| Module ID | Kind | Namespace | Capabilities | Description |
| :--- | :--- | :--- | :--- | :--- |
| `omniroute` | Built-in | `/ominroute`, `/omniroute` | `chat_completions`, `models`, `failover` | Core OpenAI-compatible reverse proxy with casing repair and multi-model failover. |
| `muse` | Built-in | `/muse-ai` | `chat`, `models`, `video`, `image` | Multi-modal AI provider with WebSocket streaming, image generation, and video rendering pipelines. |
| `jev` | Built-in | `/jev` | `decisions`, `models` | High-throughput multi-tier decision engine and OpenRouter failover provider. |
| `tiktok-studio` | External Catalog | `/custom/tiktok` | `video_download`, `items_manage`, `publish`, `studio` | TikTok Creator Studio adapter installed from [`giauphan/TikTok-Creator-Studio-Adapter`](https://github.com/giauphan/TikTok-Creator-Studio-Adapter). |

### 9.1 External Module Example: TikTok Creator Studio (`tiktok-studio`)

The TikTok Creator Studio Adapter lives in its own dedicated repository (`https://github.com/giauphan/TikTok-Creator-Studio-Adapter.git`) and is registered in the Gateway's System Module Catalog (`src/modules/catalog.rs`):

- **1-Click Install**: Install from `/dashboard#module-catalog` or `POST /api/v1/dashboard/modules/install` with `{"id": "tiktok-studio"}` (or `"source": "https://github.com/giauphan/TikTok-Creator-Studio-Adapter.git"`).
- **Automatic UI Config**: Accessible at `GET /modules/tiktok-studio/config` (or `/dashboard#provider-settings` → **TikTok Creator Studio Adapter** tab) and the module's own self-contained UI at `GET /custom/tiktok/ui`.
- **Module Endpoints** (mounted under `/tiktok` and `/custom/tiktok`):
  - `GET /tiktok/v1/status` & `GET/POST /tiktok/v1/config` — Module status and credential persistence (`config.json`, `0600`).
  - `GET /tiktok/v1/user` — Authenticated creator profile identity check.
  - `POST /tiktok/v1/config/har` — Ingests `download-tiktok.har` to extract `base_url`, `cookie`, and `tt-csrf-token`.
  - `GET /tiktok/v1/items?cursor=0&count=10` — Paginated creator video listings and engagement counters.
  - `GET /tiktok/v1/download?item_id=<id>` — Direct watermark-free CDN video download URL resolution.
  - `GET /tiktok/v1/upload/auth` & `POST /tiktok/v1/publish` — Video upload authorization and Creator Studio post publishing (see [Publishing Guide](https://github.com/giauphan/TikTok-Creator-Studio-Adapter/blob/main/docs/PUBLISHING.md)).

### 9.2 Automatic Module UI Configuration & Dynamic Dashboard Menu Positioning

Every built-in and personal module registered in `ModuleRegistry` automatically gets:

1. **Dynamic Dashboard Provider Menus & Tabs**:
   - The Dashboard **Overview** (`Provider modules`), **Sidebar** (`PINNED PROVIDERS`), **Provider Settings** tabs (`.tabline`), and **Installed Modules** table are dynamically driven by `ModuleDescriptor` metadata (`menu_position`, `menu_order`, `icon`, `description`, `config_url`, `settings_tab`, `config_schema`).
   - Installing any personal or catalog module automatically creates a configuration tab and schema-driven settings form in `/dashboard#provider-settings`.
2. **Standalone Automatic UI Configuration Page (`/modules/<id>/config`)**:
   - Visiting `GET /modules/<id>/config` (or `/modules/<id>/settings` / `/modules/<id>/config-page`) for any registered module serves `assets/module-config.html`, rendering form controls from the module's `config_schema` alongside Dashboard UI menu placement controls.
3. **Dashboard Module Position & Config REST API**:
   - `POST /api/v1/dashboard/modules/position` — Updates a module's UI placement (`"menu_position": "providers" | "sidebar" | "studios" | "overview" | "hidden"`) and priority order (`"menu_order": 10` or `"direction": "up" | "down"`), persisted to `gateway_config.json` (`module_positions`).
   - `GET /api/v1/dashboard/modules/config?id=<id>` — Returns the module's `config_schema`, UI position metadata, and current configuration with all secret fields (`api_key`, `cookie`, `token`, `csrf_token`, etc.) masked.
   - `POST /api/v1/dashboard/modules/config` — Updates per-module configuration and UI menu position, persisted to `gateway_config.json` (`module_configs`).
