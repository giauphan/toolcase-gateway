# System Design & Architecture: toolcase-gateway-rs

This document defines the system design, core pipelines, and architecture standards for `toolcase-gateway-rs`. It is grounded in the actual codebase structure and established patterns, following **Approach A: Layered Modular DDD + Pipeline Router**.

---

## 1. System Overview

`toolcase-gateway-rs` is a zero-dependency async-free HTTP/1.1 reverse proxy written in pure standard library Rust (`#![forbid(unsafe_code)]`).

- **Concurrency Model:** Thread-per-connection (`std::thread::spawn`), bounded by atomic counter `ACTIVE` against `GW_MAX_CONNECTIONS` (default: 256).
- **IO Pattern:** Synchronous blocking standard library IO (`std::net::TcpListener`, `std::net::TcpStream`) with per-socket timeouts (`GW_IO_TIMEOUT_SECS`, default: 120s). Zero `tokio` or `async-std`.
- **Streaming Strategy:** Full request buffering (up to 64 MB) to inspect/rewrite headers and body models, followed by streaming chunked response transfer to the client. Hop-by-hop headers stripped; `Accept-Encoding: identity` enforced.

---

## 2. Established Subsystems & True Pipelines

The gateway routes inbound HTTP requests through distinct subsystem pipelines via fail-closed prefix matching in `src/routes.rs`:

```
                           Inbound TCP (Port 8080)
                                      │
                                      ▼
                      Request Framing & Parsing (src/http.rs)
                                      │
                                      ▼
                        Route Dispatcher (src/routes.rs)
           ┌──────────────────────────┼──────────────────────────┐
           │                          │                          │
           ▼                          ▼                          ▼
  MuseAI Subsystem              Jev Subsystem            OmniRoute (Fallback)
  (src/museai/)                 (src/jev/)               (src/omniroute.rs)
  ├── POST /muse-ai/v1          ├── /jev/v1/*            ├── Model Normalization
  ├── POST /muse-ai/v1/create-  ├── /jev-ai/v1/*         ├── Exponential Retry
  │    video                    └── Multi-Account Pool   ├── Tool Casing Repair
  ├── DELETE /muse-ai/threads/      Rotation             └── Upstream Proxy
  │    {id}                         (src/jev/pool.rs)        (src/rewrite.rs)
  └── UI & Config
       (GET /video-template,
        GET/POST /muse-config)
```

### 2.1. OmniRoute Pipeline (`src/omniroute.rs`, `src/rewrite.rs`)
- **Purpose:** Primary upstream proxy with model alias resolution, effort suffix matching (`-high`, `-medium`, etc.), and retryable error failover.
- **Failover Statuses:** `400, 401, 402, 403, 408, 429, 500, 502, 503, 504, 524`. Rotates through `GW_FALLBACK_MODELS`.
- **Streaming Tool Repair:** Buffers chunked response bodies across line boundaries, matching casing against tools requested by client, emitting corrected tool-name casing.

### 2.2. MuseAI Subsystem (`src/museai/`)
- **Transport & Wire Protocols:**
  - `src/museai/transport.rs` (338 lines): `tungstenite`-based WebSocket transport with deadline management.
  - `src/museai/noise.rs` (211 lines): `snow` state machine handling `Noise_XX_25519_AESGCM_SHA256` and `Noise_IK` handshakes.
  - `src/museai/protocol.rs` (386 lines): Protobuf frame encoders/decoders for `NoiseTransportFrame` and `ApplicationRequest`.
- **Domain & Application Services:**
  - `src/museai/chat.rs` (930 lines — *exceeds 800-line ceiling*): Noise stream parsing, prompt normalization, and chat completion execution.
  - `src/museai/video.rs` (385 lines): Prompt construction, duration validation, and Google Drive / CDN artifact extraction.
  - `src/museai/threads.rs` (294 lines): In-memory active thread tracking, periodic background TTL cleanup worker.
  - `src/museai/session.rs` (375 lines): REST bootstrapping, VM discovery, and WebSocket URL validation.
- **Interface / HTTP:**
  - `src/museai/handlers.rs` (158 lines): HTTP routes for `/muse-ai/*`.
  - `src/museai/mod.rs` (32 lines): Thin public facade exposing crate-level re-exports.

### 2.3. Jev Subsystem (`src/jev/`)
- Multi-account rotation pool for Jev endpoints (`GW_JEV_API_KEYS`).
- Handlers in `src/jev/handlers.rs` (221 lines), Account Pool in `src/jev/pool.rs` (104 lines), Business classification in `src/jev/business.rs` (21 lines).

### 2.4. Configuration & HAR Importer (`src/har_config.rs`, `src/config.rs`)
- `src/har_config.rs` (797 lines — *approaching 800-line ceiling*): HAR JSON extraction, token/cookie sanitizer, and atomic `.env` persistence.
- `src/config.rs` (81 lines): `ConfigStore` thread-safe configuration wrapper with `Arc<RwLock<Config>>`.

---

## 3. Maintenance Target & File Size Governance

The project strictly follows the **<450 line target** and **800 line hard ceiling** specified in `AGENTS.md`.

### Existing File Size Hotspots
| File | Current Lines | Required Decomposition |
| :--- | :--- | :--- |
| `src/tests.rs` | 3,002 | Split into modular test targets under `tests/` or focused test modules |
| `src/museai/chat.rs` | 930 | Split into `chat/parser.rs` (lines 57-174, 350-355), `chat/stream.rs` (lines 174-341), `chat/client.rs` (lines 374-595), tests into `tests/museai_chat.rs` (lines 596-940) |
| `src/har_config.rs` | 797 | Split into `har/parser.rs` (lines 73-390), `har/response.rs` (lines 422-679), `har/persist.rs` (lines 703-789), tests into `tests/har_config.rs`

---

## 4. Architectural Rules for Future Code

1. **Keep Zero-Async Invariant**: Do not introduce asynchronous runtimes (`tokio`, `async-std`). All concurrency must use `std::thread::spawn` or background worker threads.
2. **Modular Facade Pattern**: Subsystems must be isolated in dedicated directories with a `mod.rs` exposing a minimal set of `pub(crate)` re-exports.
3. **Fail-Closed Route Matching**: Route prefixes must be explicitly classified in `src/routes.rs`. Generic fallback paths (`/v1/chat/completions`) must never be intercepted by dedicated platform handlers unless explicitly prefixed.
4. **No Secrets in Code**: Secrets must only be loaded from environment variables (`dotenvy`) or dynamic `ConfigStore` snapshots.
