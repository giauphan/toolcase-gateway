# Project Structure & Module Ownership

This document describes the live codebase organization, file ownership, and architectural separation of `toolcase-gateway`.
It serves as the definitive reference for human developers and autonomous AI agents modifying this repository.

---

## 1. Architectural Philosophy

`toolcase-gateway` is organized as a single, cohesive Rust crate applying **Domain-Driven Design (DDD)** and **Clean Architecture**:

- **Domain/Business Separation**: Core business rules (casing restoration, model selection, Sentinel approval evaluation, Jev account rotation) are kept strictly independent of low-level socket I/O, HTTP framing, and wire serialization.
- **Fail-Closed Centralized Dispatching**: All route matching lives exclusively in `src/routes.rs`. Individual provider modules never inspect or parse global URL paths directly.
- **Subsystem Facade Encapsulation**: Domain capabilities are organized into focused feature and provider modules. Narrow facades (`pub(crate) use` in `mod.rs`) export public capabilities, preventing external modules from depending on private internal mechanics.
- **File Size Ceilings**:
  - **Target**: Under **450 lines** per module.
  - **Hard Ceiling**: **800 lines** maximum. Files approaching 800 lines must be partitioned into cohesive submodules.
- **Zero-Async & Memory Safety**: Standard library blocking networking only; `#![forbid(unsafe_code)]` enforced everywhere.

---

## 2. Directory Tree

```text
toolcase-gateway-rs/
├── Cargo.toml                    # Manifest, dependencies, and forbid(unsafe_code) lints
├── Cargo.lock                    # Dependency lockfile
├── AGENTS.md                     # AI agent operational ground rules and lifecycle
├── CONTRIBUTING.md               # Quality gates, PR verification rules, coding standards
├── README.md                     # High-level overview, quickstart, and configuration summary
├── SECURITY.md                   # Security reporting and vulnerability disclosure
├── assets/                       # Embedded HTML/CSS/JS frontend assets
│   ├── dashboard.html            # Runtime Configuration Dashboard UI
│   ├── muse_config.html          # Legacy Muse HAR configuration UI
│   └── video_template.html       # Video Studio template UI
├── docs/                         # Canonical project documentation suite
│   ├── ARCHITECTURE.md           # Routing architecture, canonical table, and lifecycle
│   ├── CONFIGURATION.md          # Environment variables, JSON schema, and precedence
│   ├── SECURITY_MODEL.md         # Trust boundaries, threats, credential masking, 0600 I/O
│   ├── SYSTEM_DESIGN.md          # Subsystem design, runtime flow, code ownership, concurrency
│   ├── PROJECT_STRUCTURE.md      # This live directory tree and ownership reference
│   ├── JEV_INTEGRATION.md        # Jev Decision API schemas, account pool, and failovers
│   ├── MODELS.md                 # Supported model catalogs, routing rules, reasoning tiers
│   ├── muse-video-generation.md  # Muse video generation API and job lifecycle
│   └── MUSE_VIDEO_CURL_TEST.md   # Smoke test commands for Muse video workflows
├── plan/
│   ├── pending/                  # Active requirement plans (must exist before implementation)
│   └── done/                     # Archived, fully verified and completed plans
├── src/                          # Production source code
│   ├── main.rs                   # Executable entry point: CLI arguments, TCP listener loop
│   ├── lib.rs                    # Library facade and shared public interfaces
│   ├── routes.rs                 # Authoritative fail-closed route dispatcher
│   ├── config.rs                 # Config, ConfigStore, PersistentConfig, atomic 0600 JSON persistence
│   ├── http.rs                   # HTTP/1.1 framing parser, header validation, chunked encoding
│   ├── rewrite.rs                # Pure transformations: tool-name casing restoration, model substitution
│   ├── har_config.rs             # HAR capture parsing, cookie/token extraction and validation
│   ├── video_template.rs         # Asset delivery handlers for video templates and legacy UI
│   ├── omniroute.rs              # OmniRoute candidate selection, retry loop, streaming proxy
│   ├── http_provider.rs          # Reusable HTTP failover and ureq execution helpers
│   ├── test_helpers.rs           # Shared testing utilities (internal)
│   │
│   ├── runtime/                  # Connection admission and resource boundary enforcement
│   │   ├── mod.rs                # Runtime module facade
│   │   └── admission.rs          # Global in-flight request byte tracking and limits
│   │
│   ├── gateway/                  # Gateway routing policies and retry mechanics
│   │   ├── mod.rs                # Gateway policy facade
│   │   ├── candidates.rs         # Fallback candidate rotation and model resolution
│   │   ├── capabilities.rs       # Upstream capability flags and headers
│   │   ├── policy.rs             # Retry condition evaluators
│   │   └── retry.rs              # Exponential backoff and retry jitter calculations
│   │
│   ├── codecs/                   # Streaming wire codecs
│   │   ├── mod.rs                # Codec facade
│   │   └── sse.rs                # Server-Sent Events (SSE) parsing and framing
│   │
│   ├── features/                 # User-facing and operational feature subsystems
│   │   ├── mod.rs                # Features root facade
│   │   ├── dashboard/            # Interactive Management Dashboard
│   │   │   ├── mod.rs            # Dashboard feature facade
│   │   │   ├── service.rs        # Overview endpoint, account ingestion, gateway config
│   │   │   └── muse.rs           # Muse configuration endpoint and validation
│   │   ├── video/                # Video Studio orchestration
│   │   │   ├── mod.rs            # Video feature facade
│   │   │   ├── service.rs        # Provider task registration, status polling, reconciliation
│   │   │   └── types.rs          # Video task records and provider descriptors
│   │   └── proxy/                # Generic HTTP proxy pipeline
│   │       ├── mod.rs            # Proxy feature facade
│   │       └── service.rs        # Proxy request/response orchestration
│   │
│   ├── providers/                # External upstream provider integrations
│   │   ├── mod.rs                # Providers root facade
│   │   ├── omniroute/            # OmniRoute LLM proxy adapter
│   │   │   ├── mod.rs            # OmniRoute adapter facade
│   │   │   └── adapter.rs        # Chat completions invocation adapter
│   │   ├── jev/                  # Jev AI Decision API
│   │   │   ├── mod.rs            # Jev provider facade
│   │   │   ├── accounts.rs       # JevAccountPool: atomic round-robin and candidate selection
│   │   │   ├── adapter.rs        # Upstream HTTP execution and failover orchestration
│   │   │   ├── business.rs       # Retryable status evaluation (401, 402, 403, 429)
│   │   │   └── route.rs          # Upstream path resolution (/systemone, /models, /requests)
│   │   └── museai/               # Muse AI Subsystem
│   │       ├── mod.rs            # Muse provider facade
│   │       ├── handlers.rs       # HTTP endpoint adapters (/v1, /create-video, /videos)
│   │       ├── lifecycle.rs      # Session bootstrap, VM wake, and token acquisition
│   │       ├── protocol.rs       # Protobuf wire framing and binary message serialization
│   │       ├── transport.rs      # Blocking WebSocket client
│   │       ├── noise.rs          # Noise_XX_25519_AESGCM_SHA256 cryptographic handshake
│   │       ├── business.rs       # Pure business rules and video URL extraction
│   │       ├── session.rs        # Session state and credential tracking
│   │       ├── threads.rs        # Thread tracking and periodic background cleanup
│   │       ├── approval/         # Headless Sentinel auto-approval coordinator
│   │       │   ├── mod.rs        # Approval coordinator facade
│   │       │   ├── coordinator.rs# Snapshot evaluation and decision dispatch
│   │       │   ├── policy.rs     # Scope, type, and destination allowlist evaluation
│   │       │   └── protocol.rs   # Sentinel REST client and wire structures
│   │       ├── chat/             # Chat completion streaming orchestration
│   │       ├── image/            # Image creation, aspect ratios, preview, and download
│   │       └── video/            # Video job execution and durable state tracking
│   │
│   ├── museai/                   # Backwards-compatible facade
│   │   └── mod.rs                # Re-exports crate::providers::museai
│   └── jev/                      # Backwards-compatible facade
│       └── mod.rs                # Re-exports crate::providers::jev
│
├── tests/                        # Comprehensive integration test suite
│   ├── gateway.rs                # Gateway integration entry point
│   ├── gateway/                  # Gateway integration test modules
│   │   ├── core_http_tests.rs    # HTTP/1.1 framing, header injection, size bounds
│   │   ├── protocol_omniroute_tests.rs # OmniRoute fallback and casing repair
│   │   ├── har_config_tests.rs   # HAR parsing and credential persistence
│   │   ├── muse_video_route_tests.rs   # Video route contracts and parameter validation
│   │   ├── museai_live_tests.rs  # Opt-in live tests against external endpoints
│   │   └── common.rs             # Shared test harnesses and mock servers
│   ├── dashboard.rs              # Dashboard integration entry point
│   ├── dashboard/                # Dashboard integration test modules
│   │   ├── config_storage_tests.rs # Standalone JSON persistence, atomic rename, 0600 mode
│   │   └── dashboard_tests.rs    # Dashboard overview, account ingestion, validation
│   ├── jev.rs                    # Jev integration entry point
│   ├── jev/                      # Jev integration test modules
│   │   └── jev_tests.rs          # Account pool rotation, failover, idempotency, path mapping
│   ├── muse.rs                   # Muse integration entry point
│   ├── muse/                     # Muse integration test modules
│   │   ├── approval_coordinator_tests.rs # Sentinel headless approval resolution
│   │   ├── approval_policy_tests.rs      # Allowlist matching and scope rejection
│   │   ├── approval_protocol_tests.rs    # Sentinel payload serialization
│   │   ├── approval_integration_tests.rs # End-to-end approval intercept flow
│   │   ├── chat_stream_tests.rs          # Noise chat streaming and event parsing
│   │   ├── lifecycle_tests.rs            # Session acquisition and VM wake
│   │   ├── video_tests.rs                # Video job creation and state persistence
│   │   ├── threads_tests.rs              # Thread retention and cleanup
│   │   └── common.rs                     # Muse mock servers and test fixtures
│   ├── video_template.rs         # Video template UI entry point
│   ├── video_template/           # Video template test modules
│   │   └── video_template_tests.rs # Template page serving and parameter extraction
│   ├── config.rs                 # Environment and configuration unit/integration tests
│   └── routes_contract.rs        # Full contract test verifying all canonical routes
```

---

## 3. Subsystem Module Mapping

The table below maps business capabilities to their implementing modules:

| Subsystem / Capability | Primary Source Module | Supporting Modules | Tests |
| :--- | :--- | :--- | :--- |
| **Route Dispatching** | `src/routes.rs` | `src/http.rs` | `tests/routes_contract.rs` |
| **Resource Admission** | `src/runtime/admission.rs` | `src/http.rs` | `tests/gateway/core_http_tests.rs` |
| **Config Persistence** | `src/config.rs` | `src/features/dashboard/` | `tests/dashboard/config_storage_tests.rs` |
| **OmniRoute Proxy** | `src/omniroute.rs` | `src/gateway/`, `src/rewrite.rs` | `tests/gateway/protocol_omniroute_tests.rs` |
| **Dashboard UI & API** | `src/features/dashboard/` | `src/config.rs` | `tests/dashboard/dashboard_tests.rs` |
| **Jev Decision API** | `src/providers/jev/` | `src/http_provider.rs` | `tests/jev/jev_tests.rs` |
| **Muse AI CVM Protocol** | `src/providers/museai/` | `src/codecs/` | `tests/muse/chat_stream_tests.rs` |
| **Sentinel Auto-Approval** | `src/providers/museai/approval/` | `src/providers/museai/video/` | `tests/muse/approval_*_tests.rs` |
| **Video Studio** | `src/features/video/` | `src/video_template.rs` | `tests/video_template/video_template_tests.rs` |

---

## 4. Module Decomposition Rules for Developers and AI Agents

When modifying existing code or adding new functionality, obey these strict placement rules:

1. **Keep Handlers Thin**: HTTP handler functions in `src/routes.rs` or `src/features/*/` must only parse request inputs, invoke domain services, and format HTTP responses. Never embed network client logic or file I/O directly in routing match arms.
2. **Use Facade Exports**: Expose subsystem capabilities through `src/providers/<provider>/mod.rs` using `pub use` or `pub(crate) use`. Consumers should never reach deep into private submodules (e.g. use `crate::providers::jev::JevAccountPool`, not `crate::providers::jev::accounts::JevAccountPool`).
3. **Obey File Size Ceilings**:
   - Strive for `< 450 lines` per file.
   - If a file exceeds `800 lines`, decompose it into cohesive submodules grouped by responsibility under a dedicated folder with a `mod.rs` facade.
4. **Preserve Backward Compatibility**: When refactoring internal paths, maintain legacy facades (such as `src/museai/mod.rs` and `src/jev/mod.rs`) so that external tests and route bindings continue to compile cleanly.
5. **No Async Dependencies**: Use `ureq`, `std::net`, `std::sync`, and `std::thread`. Never introduce `tokio`, `futures`, or `async-std`.
