# Platform Modules & Plugin Architecture

Toolcase Gateway features an extensible, self-contained module and plugin architecture modeled after the **WordPress plugin** and **Odoo addon** paradigms.

External developers and operators can author, package, install, and run provider modules without needing the gateway source code or recompiling the binary.

---

## 1. Architectural Concept

Just like WordPress plugins (`wp-content/plugins/<plugin-slug>/`) and Odoo modules (`addons/<module_name>/` with `__manifest__.py`):

1. **Self-Contained Bundles**: Every module is an independent directory under `$GW_STATE_DIR/modules/<module-id>/` (or `./modules/<module-id>/`). It bundles its own manifest (`module.json`, `manifest.json`, or `__manifest__.json`), executables, scripts, configuration, and assets.
2. **Auto-Discovery**: The gateway automatically scans the modules directory on boot. Any valid folder containing a manifest is discovered and registered.
3. **Dedicated URL Namespace**: Each module owns an isolated public URL prefix (e.g. `/custom/my-provider/...`), completely separate from core gateway routes (`/ominroute/...`, `/muse-ai/...`, `/jev/...`).
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

---

## 4. How to Install Modules

### Option A: From GitHub Repository

Provide the GitHub URL. The installer downloads the branch archive, verifies or generates the manifest, and extracts it:

```sh
curl -X POST http://127.0.0.1:20129/api/v1/dashboard/modules/install \
  -H "Content-Type: application/json" \
  -d '{
    "source": "https://github.com/my-org/my-gateway-module",
    "branch": "main"
  }'
```

### Option B: From ZIP Archive (URL or Upload)

1. **Via URL**:
   ```sh
   curl -X POST http://127.0.0.1:20129/api/v1/dashboard/modules/install \
     -H "Content-Type: application/json" \
     -d '{
       "source": "https://example.com/downloads/my-module.zip"
     }'
   ```

2. **Via Web Dashboard Upload**:
   - Navigate to `http://127.0.0.1:20129/dashboard#modules-section`.
   - Open **Install Self-Contained Module**.
   - Select your `.zip` archive or paste a GitHub link.
   - Click **Install Module**.

3. **Via Base64 Payload**:
   ```sh
   curl -X POST http://127.0.0.1:20129/api/v1/dashboard/modules/install \
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

AI agents connected over MCP can invoke the `gateway_install_personal_module` tool:

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
   - Subprocesses execute with an isolated environment (`env_clear()`), retaining only necessary `PATH` and non-secret contextual identifiers (`MODULE_ID`, `MODULE_NAMESPACE`).
   - Standard timeouts (default 30 seconds) prevent runaway child processes.
4. **Namespace Collision Defense**:
   - The registry rejects any module attempting to claim or shadow built-in routes (`/ominroute`, `/muse-ai`, `/jev`) or system routes (`/api`, `/dashboard`, `/mcp`).
