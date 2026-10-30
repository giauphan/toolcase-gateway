# Toolcase Gateway Addons & Suggested Modules

Welcome to the **Toolcase Gateway Addons** directory.

This directory is an auto-discovered workspace root for **local drop-in personal modules and provider addons**. Any sub-directory placed here containing a valid manifest (`module.json`, `manifest.json`, or `__manifest__.json`) is automatically loaded and routed on gateway startup.

---

## 1. Suggested Catalog Modules (Ready for 1-Click Install)

The gateway includes pre-configured, production-ready provider adapters in its System Catalog (`GET /api/v1/dashboard/modules/catalog` or via `/dashboard` under **Module Catalog**):

| Module ID | Name | Source / Repo | Capabilities | Namespace |
| :--- | :--- | :--- | :--- | :--- |
| `tiktok-studio` | **TikTok Creator Studio Adapter** | [giauphan/TikTok-Creator-Studio-Adapter](https://github.com/giauphan/TikTok-Creator-Studio-Adapter.git) | `video_download`, `items_manage`, `publish`, `studio` | `/custom/tiktok` |
| `claude-adapter` | **Anthropic Claude Adapter** | Catalog Template | `chat_completions`, `models`, `streaming` | `/custom/claude` |
| `groq-fast` | **Groq High-Speed LLM** | Catalog Template | `chat_completions`, `models` | `/custom/groq` |
| `cohere-provider`| **Cohere Provider Adapter** | Catalog Template | `chat_completions`, `models` | `/custom/cohere` |
| `mistral-ai` | **Mistral AI Provider** | Catalog Template | `chat_completions`, `models` | `/custom/mistral` |
| `local-echo` | **Local Echo Test Module** | In-Tree Reference | `chat_completions` | `/custom/echo` |

To install any of these, use the web dashboard at `http://127.0.0.1:20130/dashboard` or submit a `POST /api/v1/dashboard/modules/install` request with the GitHub repository URL.

---

## 2. In-Tree Starter Templates

Looking to author a new addon? Starter kits are located in [`examples/modules/`](../examples/modules/):

- [`examples/modules/echo-provider/`](../examples/modules/echo-provider/): Minimal Python reference adapter implementing standard request-response JSON-RPC streaming.

To start a new addon from the template:
```bash
cp -r examples/modules/echo-provider addons/my-custom-addon
cd addons/my-custom-addon
```

---

## 3. Direct Drop-in Architecture

Each addon resides in its own isolated sub-directory under `addons/<id>/`:

```text
addons/
├── README.md
└── my-custom-addon/
    ├── module.json      # Declarative manifest (ID, name, namespace, entrypoint)
    ├── run.sh           # Executable launcher (chmod +x)
    └── adapter.py       # Your provider logic (Python, Node.js, Go, or Rust)
```

### Manifest Example (`module.json`)

```json
{
  "id": "my-custom-addon",
  "name": "My Custom Addon",
  "version": "1.0.0",
  "namespace": "/custom/my-addon",
  "protocol": "stdio-stream",
  "entrypoint": "run.sh",
  "capabilities": ["chat_completions", "models"]
}
```

### Auto-Discovery Lifecycle

1. When the gateway launches, `scan_and_register_modules` inspects:
   - `$GW_STATE_DIR/modules/` (runtime state directory)
   - `./modules/` (optional workspace modules)
   - `./addons/` (this workspace directory)
2. Every valid addon is registered into the thread-safe `ModuleRegistry`.
3. Requests matching `/custom/my-addon/*` are automatically dispatched to your addon with complete prefix isolation.
