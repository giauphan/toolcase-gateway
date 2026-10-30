# Platform Modules & Starter Kits

This directory contains reference starter templates for authoring **Toolcase Gateway Modules**.

Following **Clean Architecture** and the **Microkernel Pattern**, the gateway core maintains a strict boundary between the host proxy and external vendor logic.

---

## 1. Architectural Model: Starter Kits vs. Production Modules

```text
┌─────────────────────────────────────────────────────────────┐
│               toolcase-gateway-rs (Core Host)               │
│                                                             │
│   • In-Tree Starter Templates (examples/modules/)           │
│   • Dynamic Module Catalog Registry (src/modules/catalog.rs)│
│   • Runtime Module Sandboxing ($GW_STATE_DIR/modules/)      │
└──────────────────────────────┬──────────────────────────────┘
                               │ Discovers & Installs
                               ▼
┌─────────────────────────────────────────────────────────────┐
│          Decoupled External Module Repositories             │
│                                                             │
│   • TikTok Creator Studio: giauphan/TikTok-Creator-Studio-  │
│     Adapter.git                                             │
│   • Community Provider Adapters (Cohere, Claude, Groq...)   │
│   • Enterprise Private Microservices (Python, Node, Go)     │
└─────────────────────────────────────────────────────────────┘
```

1. **Starter Kits (`examples/modules/`)**:
   - In-tree code is reserved for **minimal reference templates** (such as [`echo-provider/`](./echo-provider/)).
   - They demonstrate protocol compliance (`stdio-stream` or `local-http`), manifest declarations, and error handling without adding third-party platform dependencies to the gateway codebase.

2. **Production Addons (External Git Repositories)**:
   - Full vendor implementations (e.g., [`TikTok-Creator-Studio-Adapter`](https://github.com/giauphan/TikTok-Creator-Studio-Adapter.git)) live in their own dedicated Git repositories.
   - This keeps vendor-specific scraping, SDKs, and platform churn out of the core gateway release cycle.
   - External modules are registered in the Gateway's System Catalog (`src/modules/catalog.rs`) for 1-click installation from the Web Dashboard.

---

## 2. Available Reference Starter Kits

| Template | Protocol | Capabilities | Purpose |
| :--- | :--- | :--- | :--- |
| [`echo-provider`](./echo-provider/) | `stdio-stream` | `chat_completions`, `models` | Minimal Python reference adapter showing request-response framing and JSON-RPC lifecycle. |

---

## 3. How to Create an External Module

To build a new module:

1. **Clone or copy the starter kit**:
   ```bash
   cp -r examples/modules/echo-provider my-new-adapter
   cd my-new-adapter
   ```

2. **Define `module.json`**:
   Declare your module's identity, entrypoint, namespace, and configuration schema:
   ```json
   {
     "id": "my-new-adapter",
     "name": "My Custom Adapter",
     "version": "1.0.0",
     "namespace": "/custom/my-adapter",
     "entrypoint": "run.sh",
     "protocol": "stdio-stream",
     "capabilities": ["chat_completions"],
     "config_schema": {
       "title": "My Adapter Settings",
       "fields": [
         { "key": "api_key", "label": "API Key", "type": "password", "secret": true }
       ]
     }
   }
   ```

3. **Publish to GitHub**:
   Push your repository to GitHub (e.g., `https://github.com/your-org/my-new-adapter.git`).

4. **Install in Toolcase Gateway**:
   - **Via Dashboard**: Open `/dashboard#catalog`, paste your GitHub repository URL into the **Custom Module Install** form, and click **Install Module**.
   - **Via REST API**:
     ```bash
     curl -X POST http://127.0.0.1:20128/api/v1/dashboard/modules/install \
       -H "Content-Type: application/json" \
       -d '{"source": "https://github.com/your-org/my-new-adapter.git"}'
     ```
   - **Via Catalog Suggestion**: Add your repository to `src/modules/catalog.rs` to make it available for 1-click installation by all gateway users.

---

## 4. Documentation References

- [Platform Module Specification](../../docs/MODULES.md)
- [Echo Provider Reference](../../examples/modules/echo-provider/README.md)
- [System Architecture](../../docs/ARCHITECTURE.md)
- [TikTok Creator Studio External Adapter Reference](https://github.com/giauphan/TikTok-Creator-Studio-Adapter)
