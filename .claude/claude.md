# System Design & Architecture: toolcase-gateway-rs

This document defines the system design, core pipelines, and architecture standards for `toolcase-gateway-rs`. It is grounded in the actual codebase structure and established patterns, following **Clean Architecture & Layered Modular Domain-Driven Design (DDD)**.

---

## 1. System Overview

`toolcase-gateway-rs` is a zero-async HTTP/1.1 reverse proxy, Model Context Protocol (MCP) server, and multi-provider sidecar written in pure standard library Rust (`#![forbid(unsafe_code)]`).

- **Concurrency Model:** Thread-per-connection (`std::thread::spawn`), bounded by atomic counter `ACTIVE` against `GW_MAX_CONNECTIONS` (default: 256).
- **IO Pattern:** Synchronous blocking standard library IO (`std::net::TcpListener`, `std::net::TcpStream`) with per-socket timeouts (`GW_IO_TIMEOUT_SECS`, default: 120s). Zero `tokio` or `async-std`.
- **Streaming Strategy:** Full request buffering (up to 64 MB) to inspect/rewrite headers and body models, followed by streaming chunked response transfer to the client. Hop-by-hop headers stripped; `Accept-Encoding: identity` enforced.
- **Persistence & Cryptography:** Standalone atomic JSON (`gateway_config.json`, POSIX `0600`) and embedded SQLite (`rusqlite`) with hardware-accelerated AES-256-GCM envelope encryption and blind indexing.

---

## 2. Established Subsystems & True Pipelines

The gateway routes inbound HTTP requests through distinct subsystem pipelines via fail-closed matching in `src/routes/`:

```
                           Inbound TCP (Port 20130)
                                      │
                                      ▼
                       Request Framing & Parsing (src/http.rs)
                                      │
                                      ▼
                      Route Dispatcher (src/routes/mod.rs)
         ┌───────────────────┬───────────────────┬───────────────────┐
         │                   │                   │                   │
         ▼                   ▼                   ▼                   ▼
    Dashboard UI        MCP Engine          Provider Modules     OmniRoute (Proxy)
  (src/features/      (src/api/mcp/)       (src/modules/)       (src/omniroute.rs)
    dashboard/)       ├── Stdio (--mcp)    ├── Built-in Jev     ├── Model Failover
  ├── /dashboard      ├── POST /mcp (HTTP) ├── Built-in Muse    ├── Casing Repair
  ├── /api/v1/        └── GET /mcp/sse     └── Personal Plugins └── Upstream Proxy
  │    dashboard/                              (/modules/*)
  └── Standalone & SQLite Storage
```

### 2.1. OmniRoute Pipeline (`src/omniroute.rs`, `src/rewrite.rs`)
- **Purpose:** Primary upstream proxy with model alias resolution, effort suffix matching (`-high`, `-medium`, etc.), and retryable error failover.
- **Failover Statuses:** `400, 401, 402, 403, 408, 429, 499, 500, 502, 503, 504, 524`. Rotates through `GW_FALLBACK_MODELS`.
- **Streaming Tool Repair:** Buffers chunked response bodies across line boundaries, matching casing against tools requested by client, emitting corrected tool-name casing.

### 2.2. MuseAI Subsystem (`src/museai/`)
- **Transport & Wire Protocols:**
  - `src/museai/transport.rs`: `tungstenite`-based WebSocket transport with deadline management.
  - `src/museai/noise.rs`: `snow` state machine handling `Noise_XX_25519_AESGCM_SHA256` and `Noise_IK` handshakes.
  - `src/museai/protocol.rs`: Protobuf frame encoders/decoders for `NoiseTransportFrame` and `ApplicationRequest`.
- **Domain & Application Services:**
  - `src/museai/chat/`: Conversational chat stream parsing, prompt normalization, multi-turn continuation (`reply_thread_id`).
  - `src/museai/video/`: Video prompt construction, duration validation, and CDN artifact extraction.
  - `src/museai/image/`: Text-to-image generation, aspect ratio formatting (`1:1`, `16:9`, `9:16`, `4:3`, `3:2`), preview, and download.
  - `src/museai/approval/`: Headless Sentinel auto-approval coordinator with scope and destination allowlist policies.
  - `src/museai/threads.rs`: In-memory active thread tracking, periodic background TTL cleanup worker.
  - `src/museai/session.rs`: REST bootstrapping, VM discovery, and WebSocket URL validation.

### 2.3. Jev Decision Subsystem (`src/modules/builtins/jev/`)
- Multi-account failover pool (`JevAccountPool`) supporting direct Jev API keys, OpenRouter Bearer API keys, and browser session cookies.
- 4-Tier failover cascade: Direct Jev $\rightarrow$ Jev OpenRouter $\rightarrow$ Different Decision $\rightarrow$ OpenRouter Browser Cookie.
- Atomic cooldown demotion on credit limits (`402`) or rate limits (`429`).

### 2.4. Model Context Protocol (MCP) Control Plane (`src/api/mcp/`, `src/features/mcp/`)
- Full MCP support across Stdio (`--mcp`), SSE (`GET /mcp/sse`), and Streamable HTTP (`POST /mcp`).
- Built-in tools for module cataloging, installing personal modules from local dirs/ZIP/GitHub, state toggling, and system telemetry metrics.

### 2.5. Storage & Cryptography (`src/storage/`, `src/config/`)
- `src/storage/crypto.rs`: AES-256-GCM envelope encryption and deterministic HMAC-SHA256 blind indexing for credential privacy.
- `src/storage/sqlite.rs`: Embedded SQLite database (`rusqlite`) for encrypted credential and module state storage.
- `src/config/persistent.rs`: Standalone atomic `gateway_config.json` persistence with restricted POSIX `0600` permissions.

---

## 3. Maintenance Target & File Size Governance

The project strictly enforces the **<450 line target** and **800 line hard ceiling** specified in `AGENTS.md`.

### Decomposed Subsystems & Modules
All monolithic historical hotspots have been successfully decomposed into cohesive submodules:
- `src/tests.rs` (3,002 lines) $\rightarrow$ Decomposed into focused integration test crates in `tests/` (`gateway.rs`, `dashboard.rs`, `jev.rs`, `muse.rs`, `mcp_transports_tests.rs`, `module_groups.rs`, etc.).
- `src/routes.rs` (430 lines) $\rightarrow$ Decomposed into `src/routes/` (`mod.rs`, `dashboard.rs`, `jev.rs`, `muse.rs`, `modules.rs`, `templates.rs`).
- `src/config.rs` (420 lines) $\rightarrow$ Decomposed into `src/config/` (`mod.rs`, `types.rs`, `store.rs`, `persistent.rs`, `env.rs`).
- `src/har_config.rs` (797 lines) $\rightarrow$ Decomposed into `src/features/har/` (`mod.rs`, `parser.rs`, `handler.rs`, `storage.rs`, `types.rs`).
- `src/runtime/logger/` $\rightarrow$ Decomposed into `mod.rs`, `writer.rs`, `service.rs`, `format.rs`, `types.rs`, `archive.rs` (with ZIP log rotation).

---

## 4. Architectural Rules for Future Code

1. **Keep Zero-Async Invariant**: Do not introduce asynchronous runtimes (`tokio`, `async-std`). All concurrency must use `std::thread::spawn` or background worker threads.
2. **Modular Facade Pattern**: Subsystems must be isolated in dedicated directories with a `mod.rs` exposing a minimal set of `pub(crate)` re-exports.
3. **Fail-Closed Route Matching**: Route prefixes must be explicitly classified in `src/routes/`. Generic fallback paths (`/v1/chat/completions`) must never be intercepted by dedicated platform handlers unless explicitly prefixed.
4. **No Secrets in Code or Logs**: Secrets must only be loaded from environment variables (`dotenvy`), encrypted SQLite, or dynamic `ConfigStore` snapshots, and masked in all outputs.
5. **Continuous Supply Chain Verification**: Keep Cargo dependencies and GitHub Actions audited via Dependabot.
