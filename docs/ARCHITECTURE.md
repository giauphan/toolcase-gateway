# Architecture

`toolcase-gateway` is a high-performance, zero-async, single-binary HTTP/1.1 reverse proxy written in 100% safe Rust (`#![forbid(unsafe_code)]`). It utilizes blocking I/O (`std::net` and `std::thread`) with admission control and no asynchronous runtime (zero `tokio`/`async-std`).

---

## 1. Subsystem Architecture

The gateway coordinates six cohesive subsystems behind an admission controller and exclusive, fail-closed router:

```mermaid
flowchart TD
    Client["Client Request"] --> Adm["Admission Controller & Quota Guard"]
    Adm --> Parser["HTTP Framing & Header Parser"]
    Parser --> Router{"Route Dispatcher (src/routes/)"}

    Router -->|"/dashboard, /api/v1/dashboard/*"| Dash["Dashboard & Config Engine (src/features/dashboard/)"]
    Router -->|"/mcp, /mcp/sse"| MCP["Model Context Protocol Engine (src/api/mcp/)"]
    Router -->|"/jev/*, /jev-ai/*"| Jev["Jev AI Decision Provider (src/modules/builtins/jev/)"]
    Router -->|"/muse-ai/*, /video-template, /muse-config"| Muse["Muse AI CVM Subsystem (src/museai/)"]
    Router -->|"/modules/*, /custom/*"| Mod["Dynamic Modules & Groups (src/modules/personal/)"]
    Router -->|"/omniroute, /v1/*"| Omni["OmniRoute Upstream Proxy (src/omniroute.rs)"]

    Dash --> Storage[("Standalone JSON Store (gateway_config.json)")]
    Dash & Mod --> SQLite[("Encrypted SQLite DB (AES-256-GCM Storage)")]
    Jev --> JevPool["Account Pool & Round-Robin Rotation"]
    JevPool --> JevUpstream["Jev Decision API / OpenRouter"]

    Muse --> Noise["Noise_XX CVM WebSocket"]
    Muse --> Sentinel["Muse Sentinel Auto-Approval Coordinator"]
    Muse --> VideoLifecycle["Video Generation & Job Store"]

    Omni --> Candidates["Candidate Model Planner"]
    Candidates --> Rewriter["Tool-Name Case Rewriter"]
    Rewriter --> LLMUpstream["OpenAI-Compatible Upstream"]
```

---

## 2. Route Ownership & Invariants

Dispatch in `src/routes/` is **exclusive and fail-closed**: a request matching a subsystem namespace is strictly handled or rejected within that subsystem. It **never falls through** to the generic OmniRoute proxy. Unknown paths under reserved namespaces return `404 Not Found`; mismatched HTTP methods return `405 Method Not Allowed`.

| Route | Subsystem / Owner | Allowed Methods | Purpose |
| :--- | :--- | :--- | :--- |
| `/dashboard`, `/dashboard/` | Dashboard UI | `GET` | Interactive pipeline, provider pool, and configuration dashboard |
| `/sw.js`, `/manifest.webmanifest` | Dashboard PWA | `GET` | Service worker and web app manifest for dashboard PWA |
| `/api/v1/dashboard/overview` | Dashboard API | `GET` | Aggregated provider status, masked credentials, and live config |
| `/api/v1/dashboard/config` | Dashboard API | `POST` | Update gateway parameters (host, port, fallbacks, timeout) |
| `/api/v1/dashboard/muse-config` | Dashboard API | `POST` | Update Muse AI endpoints, tokens, cookies, approvals, video prompt |
| `/api/v1/dashboard/accounts` | Dashboard API | `POST` | Ingest multi-API keys/accounts into live rotation pools |
| `/api/v1/dashboard/jev-config` | Dashboard API | `POST` | Update Jev AI base URLs, OpenRouter models, and cooldowns |
| `/mcp` | MCP Transport | `POST` | Model Context Protocol Streamable HTTP endpoint for AI agents |
| `/mcp/sse` | MCP Transport | `GET` | Model Context Protocol Server-Sent Events (SSE) stream |
| `/api/v1/modules` | Module Registry | `GET` | List all discovered, installed, and built-in provider modules |
| `/api/v1/modules/install` | Module Registry | `POST` | Dynamically install module from local path, ZIP, or GitHub repo |
| `/api/v1/modules/state` | Module Registry | `POST` | Enable or disable an installed module |
| `/api/v1/modules/{id}` | Module Registry | `DELETE` | Uninstall and purge a personal provider module |
| `/modules/*`, `/custom/*` | Personal Modules | Any | Dedicated URL prefix routed to dynamic out-of-tree personal modules |
| `/api/v1/dashboard/muse-config` | Dashboard API | `POST` | Update Muse AI endpoints, tokens, cookies, approvals, video prompt |
| `/api/v1/dashboard/accounts` | Dashboard API | `POST` | Ingest multi-API keys/accounts into live rotation pools |
| `/jev/models`, `/jev/v1/models` | Jev Decision | `GET` | Supported Jev decision model catalog |
| `/jev/v1/systemone` | Jev Decision | `POST` | Jev Decision API (SystemOne schema: `state`, `questions`) |
| `/jev/v1/requests/{id}` | Jev Decision | `GET` | Decision request status lookup |
| `/jev/v1/requests/{id}/cancel` | Jev Decision | `POST` | Decision request cancellation |
| `/jev-ai/v1/*` | Jev Decision | `*` | Compatibility alias for `/jev/v1/*` |
| `/models`, `/v1/models` | OmniRoute | `GET` | OmniRoute upstream model catalog |
| `/v1/chat/completions` | OmniRoute | `POST` | OpenAI-compatible chat completions with model failover |
| `/prism-openai/v1/chat/completions` | Prism | `POST` | Direct Prism raw OpenAI completion endpoint |
| `/muse-ai/models`, `/muse-ai/v1/models` | Muse AI | `GET` | Local Muse model catalog (`muse`) |
| `/muse-ai/v1/chat/completions` | Muse AI | `POST` | Dedicated Muse chat completions (see [docs/muse-chat.md](muse-chat.md)) |
| `/muse-ai/v1` | Muse AI | `POST` | Raw Muse REST proxy endpoint |
| `/muse-ai/v1/create-image`, `/muse-ai/create-image`, `/v1/images/generations` | Muse Image | `POST` | Text-to-image creation with aspect ratio choices (`1:1`, `16:9`, etc.) |
| `/muse-ai/v1/images/{id}` | Muse Image | `GET` | Image job status polling and metadata |
| `/muse-ai/v1/images/{id}/content` | Muse Image Media | `GET`, `HEAD` | Preview image inline |
| `/muse-ai/v1/images/{id}/download` | Muse Image Media | `GET`, `HEAD` | Download image file |
| `/image-template` | Image Studio UI | `GET`, `HEAD` | Interactive image generation studio with aspect ratio pills, live preview, and download |
| `/muse-ai/v1/create-video` | Muse Video | `POST` | Video creation task submission |
| `/muse-ai/v1/videos/{id}` | Muse Video | `GET`, `POST` | Video job polling (`GET`) or edit prompt revision (`POST`) |
| `/muse-ai/v1/threads/{id}/cleanup` | Muse Threads | `POST` | Background worker thread registration and cleanup |
| `/muse-ai/v1/config` | Muse Config | `GET` | Effective masked Muse configuration snapshot |
| `/muse-ai/v1/config/har` | Muse Config | `POST` | DevTools HAR capture upload and credential extraction |
| `/muse-config` | Muse Config UI | `GET` | Web interface for DevTools HAR upload and import |
| `/modules/{id}/config` | Module Auto-UI Config | `GET` | Automatic schema-driven configuration & UI position page for any module |
| `/api/v1/dashboard/modules/position` | Dashboard Module UI | `POST` | Update module menu placement (`providers`, `sidebar`, `studios`, `hidden`) and order |
| `/api/v1/dashboard/modules/config` | Dashboard Module UI | `GET`, `POST` | Read or update per-module configuration with secret masking |
| `/video-template[/<name>]` | Video Studio UI | `GET` | Interactive video generation studio and template viewer |
| *All unclaimed paths* | OmniRoute Proxy | Any | Generic reverse proxy forwarding with model failover |

---

## 3. Core Subsystems

### A. Dashboard & Standalone Persistent Storage (`src/features/dashboard/`)
- **Single Page Application UI**: Served from [`assets/dashboard.html`](file:///home/ubuntu/toolcase-gateway-rs/assets/dashboard.html) at `/dashboard`. Zero external CDN or script dependencies.
- **Standalone JSON Store (`gateway_config.json`)**: Configuration is managed via [`PersistentConfig`](file:///home/ubuntu/toolcase-gateway-rs/src/config.rs). When updated from the dashboard or API, changes are applied immediately to the in-memory [`ConfigStore`](file:///home/ubuntu/toolcase-gateway-rs/src/config.rs) and persisted atomically via `fs::write` to a temporary file (`.gateway_config.json~tmp`), unix permissions `0600`, and `fs::rename`. The `.env` file is left untouched.
- **Credential Redaction**: API keys, bearer tokens, and session cookies are strictly masked (`mask_key`) in read responses (`sk-glm...d74b` or `***`). Full secrets are never emitted in API responses or log output.

### B. Jev AI Decision Engine & Account Pool (`src/providers/jev/`)
- **Decision Schema Handling**: Jev is a specialized decision service (using `state` and `questions` payload), not an OpenAI chat completion endpoint. OmniRoute chat transformations do not apply.
- **Multi-Account Rotation Pool (`JevAccountPool`)**: Rotates multiple configured API keys round-robin per request.
- **Automatic Account Failover**: Automatically rotates to the next candidate key when account errors occur (`401 Unauthorized`, `402 Payment Required`, `403 Forbidden`, `429 Rate Limit`).
- **Idempotency**: The `Idempotency-Key` HTTP header is preserved across account failovers to avoid duplicate billable decision execution.

### C. Muse AI Subsystem & Confidential VM Pipeline (`src/museai/`)
- **Noise Protocol Handshake**: End-to-end encrypted transport to Confidential VMs via `Noise_XX_25519_AESGCM_SHA256` (`src/museai/noise.rs`).
- **Sentinel Headless Auto-Approval Coordinator**: Manages auto-approval policies for outbound network requests without requiring interactive browser automation. Modes: `off` (fail-closed, prompts browser approval), `observe` (logs decisions), and `on` (`scoped-auto`, automatically approves destinations matching `allowed_destinations`).
- **Video Generation & Polling**: Handles video task submission, prompt revision tracking, artifact verification (`video_url`), and background thread cleanup (`src/museai/video.rs`, `src/museai/threads.rs`).

### D. OmniRoute Reverse Proxy (`src/omniroute.rs`, `src/rewrite.rs`)
- **Candidate Model Planning**: Sequential candidate failover for transient or retryable errors (`401, 402, 403, 408, 429, 499, 500, 502, 503, 504, 524`). Non-retryable errors (`400 Bad Request`, `413 Payload Too Large`) fail immediately without consuming candidates.
- **Tool-Name Case Repair**: Scans request tool definitions and restores exact casing in streamed responses (e.g. `bash` -> `Bash`) to repair casing mutations introduced by certain upstream models.

### E. Model Context Protocol (MCP) Control Plane (`src/api/mcp/`, `src/features/mcp/`)
- **Triple Transport Support**: Stdio JSON-RPC 2.0 (via CLI `--mcp` flag), Server-Sent Events (`GET /mcp/sse`), and Streamable HTTP (`POST /mcp`).
- **Autonomous Toolset**: Exposes tools for listing modules, installing remote GitHub/ZIP packages, toggling module states, and querying live memory/traffic metrics.
- **Agent Interoperability**: Seamless integration with modern AI engineering environments (Claude Desktop, Cursor, Windsurf, Antigravity).

### F. Dynamic Personal Modules & Plugin Architecture (`src/modules/personal/`, `src/gateway/installer.rs`)
- **Modular Plugin Paradigm**: Discovers self-contained modules located in `.gw_state/modules/<id>/`.
- **Manifest Engine**: Supports declarative `module.json`, `manifest.json`, or `__manifest__.json` formats defining entrypoint scripts, protocols, and URL namespaces.
- **Sub-Provider Groups**: Modules can declare multiple sub-providers mounted under subpaths, completely isolated from core system routes.

### G. Encrypted Storage & Cryptographic Primitives (`src/storage/`)
- **AES-256-GCM Envelope Encryption**: Credentials, API keys, and session cookies are encrypted before disk writes using hardware-accelerated AES-GCM.
- **Deterministic Blind Indexing**: Uses HMAC-SHA256 blind indexing to allow constant-time credential lookups without decrypting the entire database.
- **Embedded SQLite Engine**: Backed by `rusqlite` with the bundled SQLite engine for zero-configuration, robust ACID storage alongside the standalone `gateway_config.json`.

---

## 4. Module Map & Responsibilities

The codebase follows strict Clean Architecture & Domain-Driven Design (DDD) with clear boundaries:

| Path | Purpose / Domain | Line Target |
| :--- | :--- | :--- |
| `src/main.rs` | CLI executable entry point, signals, server loop, MCP stdio dispatch | <150 lines |
| `src/lib.rs` | Public library entry, bootstrap, and store initialization | <180 lines |
| `src/config/` | Decomposed configuration: types, store, persistent JSON, and env resolution | <450 lines/module |
| `src/routes/` | Modular fail-closed route dispatchers (dashboard, jev, muse, modules, templates) | <450 lines/module |
| `src/api/mcp/` | MCP protocol framing, SSE streams, and streamable HTTP transports | <450 lines/module |
| `src/features/mcp/` | MCP tool catalog, system metrics handler, and resource definitions | <450 lines/module |
| `src/features/dashboard/` | Dashboard overview, gateway settings API, account ingestion, and module UI | <450 lines/module |
| `src/features/har/` | HAR capture parser, credential extractor, and token masking | <450 lines/module |
| `src/features/proxy/` | Prefix rewriting and reverse proxy helpers | <450 lines/module |
| `src/modules/` | Clean Architecture module engine: builtins (jev, muse), personal, registry, catalog | <450 lines/module |
| `src/museai/` | Muse AI subsystem: transport, noise, chat, video, approval, session | Decomposed <450/module |
| `src/gateway/` | Candidate planning, retry classification, policy rules, archive extractor, module installer | Decomposed <450/module |
| `src/storage/` | AES-256-GCM envelope encryption, blind indexing, and SQLite persistence | <450 lines/module |
| `src/runtime/` | Admission controller, byte guard, connection limits, and logger (with ZIP compression) | Decomposed <450/module |
| `src/codecs/` | SSE and chunked streaming decoders | Decomposed <450/module |

For details on authoring, installing, and auto-discovering external provider modules, consult [`docs/MODULES.md`](MODULES.md).

---

## 5. Architectural Invariants

1. **Zero External Async Runtime**: All I/O uses blocking `std::net::TcpStream` with thread-per-connection pools bounded by `GW_MAX_CONNECTIONS`. No `tokio`, `async-std`, or hidden event loops.
2. **`#![forbid(unsafe_code)]`**: Enforced compiler-wide in `Cargo.toml`. Zero unsafe blocks across the entire codebase.
3. **Embedded Local Storage**: Zero dependency on external database servers or cloud databases. Persistence is strictly local, using atomic `0600` JSON files and embedded SQLite (`rusqlite`) with AES-256-GCM encryption.
4. **Strict Modular Line Ceilings**: Target under 450 lines per module (soft ceiling 800 lines maximum). Files exceeding this threshold must be partitioned into cohesive submodules with facade re-exports.
5. **Fail-Closed Security**: Network bounds, URL schemes, and route namespaces fail closed by default.
6. **Continuous Dependency Auditing**: Maintained with active Dependabot security checks for Cargo dependencies and GitHub Actions.
