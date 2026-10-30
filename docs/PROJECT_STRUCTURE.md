# Project Structure & Module Ownership

This document describes the live codebase organization, file ownership, and architectural separation of `toolcase-gateway`.
It serves as the definitive reference for human developers and autonomous AI agents modifying this repository.

---

## 1. Architectural Philosophy

`toolcase-gateway` is organized as a single, cohesive Rust crate applying **Domain-Driven Design (DDD)** and **Clean Architecture**:

- **Domain/Business Separation**: Core business rules (casing restoration, model selection, Sentinel approval evaluation, Jev account rotation, module management) are kept strictly independent of low-level socket I/O, HTTP framing, and wire serialization.
- **Fail-Closed Centralized Dispatching**: All route matching lives in modular route dispatchers under `src/routes/`. Individual provider modules never inspect or parse global URL paths directly.
- **Subsystem Facade Encapsulation**: Domain capabilities are organized into focused feature and provider modules. Narrow facades (`pub(crate) use` in `mod.rs`) export public capabilities, preventing external modules from depending on private internal mechanics.
- **File Size Ceilings**:
  - **Target**: Under **450 lines** per module.
  - **Hard Ceiling**: **800 lines** maximum. Files approaching 800 lines must be partitioned into cohesive submodules.
- **Zero-Async & Memory Safety**: Standard library blocking networking only; `#![forbid(unsafe_code)]` enforced everywhere.
- **Cryptographic & Persistent Storage**: AES-256-GCM envelope encryption and blind indexing in embedded SQLite (`rusqlite`) alongside atomic POSIX `0600` standalone JSON configuration (`gateway_config.json`).

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
├── .github/                      # GitHub workflows and automation
│   ├── dependabot.yml            # Automated weekly dependency updates (Cargo + Actions)
│   └── workflows/                # CI and build workflows
├── assets/                       # Embedded HTML/CSS/JS frontend assets
│   ├── dashboard.html            # Runtime Configuration Dashboard UI
│   ├── module-config.html        # Automatic schema-driven Module UI Configuration (/modules/<id>/config)
│   ├── muse-config.html          # Muse HAR configuration UI (/muse-config)
│   ├── image-template.html       # Image Studio template UI (/image-template)
│   ├── video-template.html       # Video Studio template UI (/video-template)
│   └── ui/                       # Dashboard CSS and JS modules
├── docs/                         # Canonical project documentation suite
│   ├── ARCHITECTURE.md           # Routing architecture, canonical table, and lifecycle
│   ├── CONFIGURATION.md          # Environment variables, JSON schema, and precedence
│   ├── SECURITY_MODEL.md         # Trust boundaries, threats, credential masking, 0600 I/O
│   ├── SYSTEM_DESIGN.md          # Subsystem design, runtime flow, code ownership, concurrency
│   ├── PROJECT_STRUCTURE.md      # This live directory tree and ownership reference
│   ├── MODULES.md                # Platform modules, manifests, and dynamic installation
│   ├── JEV_INTEGRATION.md        # Jev Decision API schemas, account pool, and failovers
│   ├── MODELS.md                 # Supported model catalogs, routing rules, reasoning tiers
│   ├── muse-chat.md              # Conversational chat completions and thread continuity
│   ├── muse-image-generation.md  # Muse image creation, aspect ratios, preview, and download
│   ├── muse-video-generation.md  # Muse video generation API and job lifecycle
│   └── MUSE_VIDEO_CURL_TEST.md   # Smoke test commands for Muse video workflows
├── plan/
│   ├── pending/                  # Active requirement plans (must exist before implementation)
│   └── done/                     # Archived, fully verified and completed plans
├── src/                          # Production source code
│   ├── main.rs                   # Executable entry point: CLI arguments, TCP listener loop, --mcp mode
│   ├── lib.rs                    # Library facade and shared public interfaces
│   ├── http.rs                   # HTTP/1.1 framing parser, header validation, chunked encoding
│   ├── rewrite.rs                # Pure transformations: tool-name casing restoration, model substitution
│   ├── omniroute.rs              # OmniRoute candidate selection, retry loop, streaming proxy
│   ├── video_template.rs         # Asset delivery handlers for video templates and legacy UI
│   ├── test_helpers.rs           # Shared testing utilities (internal)
│   │
│   ├── config/                   # Dual-layer configuration subsystem (<450 lines/module)
│   │   ├── mod.rs                # Facade and public re-exports
│   │   ├── types.rs              # Config struct, defaults, and option mapping
│   │   ├── store.rs              # Thread-safe ConfigStore (RwLock) and live pool sync
│   │   ├── persistent.rs         # Standalone JSON persistence (0600, atomic swap)
│   │   └── env.rs                # Environment variable parsing and bootstrap
│   │
│   ├── routes/                   # Canonical fail-closed route dispatchers (<450 lines/module)
│   │   ├── mod.rs                # Dispatcher orchestrator and admission checks
│   │   ├── dashboard.rs          # Dashboard UI, overview, config mutations, and PWA routes
│   │   ├── jev.rs                # Jev Decision API and catalog routing
│   │   ├── muse.rs               # Muse AI chat, video, image, and Sentinel routes
│   │   ├── modules.rs            # Dynamic personal module routing and built-in dispatch
│   │   └── templates.rs          # Template UI and static asset routes
│   │
│   ├── api/                      # External protocol APIs (<450 lines/module)
│   │   ├── mod.rs                # API facade
│   │   └── mcp/                  # Model Context Protocol (MCP) server engine
│   │       ├── mod.rs            # MCP dispatcher facade
│   │       ├── protocol.rs       # JSON-RPC 2.0 message structures and framing
│   │       ├── transport.rs      # Stdio and Streamable HTTP (POST /mcp) handlers
│   │       └── sse.rs            # Server-Sent Events (GET /mcp/sse) session registry
│   │
│   ├── storage/                  # Encrypted persistence and cryptographic primitives
│   │   ├── mod.rs                # Storage facade
│   │   ├── crypto.rs             # AES-256-GCM envelope encryption and blind indexing
│   │   └── sqlite.rs             # Embedded SQLite database (rusqlite) for account keys & modules
│   │
│   ├── modules/                  # Extensible platform module system (modular plugin style)
│   │   ├── mod.rs                # Modules facade
│   │   ├── catalog.rs            # Public module catalog and installation sources
│   │   ├── manifest.rs           # Manifest parser (module.json, manifest.json, __manifest__.json)
│   │   ├── registry.rs           # Thread-safe module registry and route prefix index
│   │   ├── validation.rs         # Module ID, namespace, and entrypoint validators
│   │   ├── builtins/             # Core built-in provider modules (Jev, Muse, OmniRoute)
│   │   ├── ui_config.rs          # Dynamic UI menu positioning and automatic module config storage
│   │   └── personal/             # Out-of-tree personal modules and subprocess runners
│   │
│   ├── museai/                   # Muse AI Subsystem (<450 lines/module)
│   │   ├── mod.rs                # Muse provider facade
│   │   ├── transport.rs          # Blocking WebSocket client with timeout guard
│   │   ├── noise.rs              # Noise_XX_25519_AESGCM_SHA256 cryptographic handshake
│   │   ├── protocol.rs           # Protobuf wire framing and binary message serialization
│   │   ├── session.rs            # Session state and credential tracking
│   │   ├── threads.rs            # Thread tracking and periodic background cleanup
│   │   ├── chat/                 # Conversational chat completion streaming
│   │   ├── image/                # Image creation, aspect ratios, preview, and download
│   │   ├── video/                # Video job execution and durable state tracking
│   │   └── approval/             # Headless Sentinel auto-approval coordinator
│   │
│   ├── features/                 # User-facing and operational feature subsystems
│   │   ├── mod.rs                # Features root facade
│   │   ├── dashboard/            # Interactive Management Dashboard
│   │   ├── har/                  # HAR capture parsing, cookie/token extraction and validation
│   │   ├── mcp/                  # MCP tool implementations (module installer, metrics)
│   │   ├── proxy/                # Reverse proxy helpers and URL prefix rewriting
│   │   └── video/                # Video studio feature handlers
│   │
│   ├── gateway/                  # Gateway routing policies and retry mechanics
│   │   ├── mod.rs                # Gateway policy facade
│   │   ├── candidates.rs         # Fallback candidate rotation and model resolution
│   │   ├── capabilities.rs       # Upstream capability flags and headers
│   │   ├── policy.rs             # Retry condition evaluators
│   │   ├── retry.rs              # Exponential backoff and retry jitter calculations
│   │   ├── archive.rs            # ZIP archive deflation and traversal security checks
│   │   └── installer.rs          # Module installer from local dir, ZIP, or GitHub repo
│   │
│   ├── runtime/                  # Connection admission and resource boundary enforcement
│   │   ├── mod.rs                # Runtime module facade
│   │   ├── admission.rs          # Global in-flight request byte tracking and limits
│   │   └── logger/               # Structured logging with daily rotation and ZIP archiving
│   │       ├── mod.rs            # Logger facade
│   │       ├── writer.rs         # Multi-file log writer
│   │       ├── service.rs        # Logger service lifecycle
│   │       ├── format.rs         # Standard log formatting and secret redaction
│   │       ├── types.rs          # Log level and stream definitions
│   │       └── archive.rs        # ZIP compression when log files exceed 10 MB
│   │
│   └── codecs/                   # Streaming wire codecs
│       ├── mod.rs                # Codec facade
│       └── sse.rs                # Server-Sent Events (SSE) parsing and framing
│   ├── museai/                   # Facade re-exporting crate::modules::builtins::museai
│   │   └── mod.rs                # Backward compatibility facade
│   ├── jev/                      # Facade re-exporting crate::modules::builtins::jev
│   │   └── mod.rs                # Backward compatibility facade
│   └── providers/                # Facade re-exporting crate::modules
│       └── mod.rs                # Backward compatibility facade
│
├── tests/                        # Comprehensive integration test suite (400+ tests)
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
│   ├── mcp_transports_tests.rs   # MCP Stdio, SSE, and Streamable HTTP tests
│   ├── module_groups.rs          # Module group isolation and prefix validation tests
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
│   ├── logger.rs                 # Multi-stream logger entry point (tests/logger/*.rs <400 lines)
│   ├── modules.rs                # Module installer & registry entry point (tests/modules/*.rs <400 lines)
│   └── routes_contract.rs        # Full contract test verifying all canonical routes
```

---

## 3. Subsystem Module Mapping

The table below maps business capabilities to their implementing modules:

| Subsystem / Capability | Primary Source Module | Supporting Modules | Tests |
| :--- | :--- | :--- | :--- |
| **Route Dispatching** | `src/routes/mod.rs` | `src/http.rs` | `tests/routes_contract.rs` |
| **Resource Admission** | `src/runtime/admission.rs` | `src/http.rs` | `tests/gateway/core_http_tests.rs` |
| **Config Persistence** | `src/config/` | `src/storage/` | `tests/dashboard/config_storage_tests.rs` |
| **OmniRoute Proxy** | `src/omniroute.rs` | `src/gateway/`, `src/rewrite.rs` | `tests/gateway/protocol_omniroute_tests.rs` |
| **Dashboard UI & API** | `src/features/dashboard/` | `src/config/` | `tests/dashboard/dashboard_tests.rs` |
| **MCP Control Plane** | `src/api/mcp/` | `src/features/mcp/` | `tests/mcp_transports_tests.rs` |
| **Dynamic Modules & Groups**| `src/modules/` | `src/gateway/installer.rs` | `tests/module_groups.rs` |
| **Encrypted SQLite Store** | `src/storage/sqlite.rs` | `src/storage/crypto.rs` | `src/storage/sqlite.rs` unittests |
| **Jev Decision API** | `src/modules/builtins/jev/` | `src/http_provider.rs` | `tests/jev/jev_tests.rs` |
| **Muse AI CVM Protocol** | `src/museai/` | `src/codecs/` | `tests/muse/chat_stream_tests.rs` |
| **Sentinel Auto-Approval** | `src/museai/approval/` | `src/museai/video/` | `tests/muse/approval_*_tests.rs` |
| **Video Studio** | `src/features/video/` | `src/video_template.rs` | `tests/video_template/video_template_tests.rs` |

---

## 4. Module Decomposition Rules for Developers and AI Agents

When modifying existing code or adding new functionality, obey these strict placement rules:

1. **Keep Handlers Thin**: HTTP handler functions in `src/routes/` or `src/features/*/` must only parse request inputs, invoke domain services, and format HTTP responses. Never embed network client logic or file I/O directly in routing match arms.
2. **Use Facade Exports**: Expose subsystem capabilities through `src/modules/<module>/mod.rs` using `pub use` or `pub(crate) use`. Consumers should never reach deep into private submodules (e.g. use `crate::modules::builtins::jev::JevAccountPool`).
3. **Obey File Size Ceilings**:
   - Strive for `< 450 lines` per file.
   - If a file exceeds `800 lines`, decompose it into cohesive submodules grouped by responsibility under a dedicated folder with a `mod.rs` facade.
4. **Preserve Backward Compatibility**: When refactoring internal paths, maintain legacy facades (such as `src/museai/mod.rs`, `src/jev/mod.rs`, and `src/providers/mod.rs`) so that external tests and route bindings continue to compile cleanly.
5. **No Async Dependencies**: Use `ureq`, `std::net`, `std::sync`, and `std::thread`. Never introduce `tokio`, `futures`, or `async-std`.
6. **Continuous Security Auditing**: Ensure any lockfile dependency additions or changes are scanned against GHSA/RustSec advisories and maintain Dependabot compatibility.
