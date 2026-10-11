# Proposed project folder structure

Status: design for owner approval; proposed paths below have not been created or migrated.

## Architecture choice

Use a single Rust crate with feature-owned modules and separation of business rules, application orchestration, and I/O inside each feature. Keep the existing Muse and Jev module names. Avoid a repository-wide `controllers/services/repositories` hierarchy that scatters each provider across unrelated folders. Do not add workspace crates or abstractions solely to move files.

## Production and repository layout

```text
toolcase-gateway-rs/
├── Cargo.toml
├── Cargo.lock
├── AGENTS.md
├── CONTRIBUTING.md
├── README.md
├── SECURITY.md
├── src/
│   ├── main.rs                 # Executable entry only
│   ├── lib.rs                  # Library entry and intentional public API
│   ├── runtime.rs              # Startup, listener, admission and worker lifecycle
│   ├── config.rs               # Shared configuration and runtime snapshots
│   ├── routes.rs               # Exclusive route dispatch, not provider logic
│   ├── http/
│   │   ├── mod.rs              # Stable HTTP facade
│   │   ├── request.rs          # Request parsing and validation
│   │   ├── response.rs         # Response headers and error writing
│   │   └── framing.rs          # Body framing and size limits
│   ├── omniroute/
│   │   ├── mod.rs              # Stable proxy facade
│   │   ├── models.rs           # Candidate selection and model normalization
│   │   ├── proxy.rs            # Forwarding and retry orchestration
│   │   └── stream.rs           # Upstream response streaming
│   ├── rewrite.rs              # Pure request/response transformations
│   ├── har_config/
│   │   ├── mod.rs              # Stable configuration-import facade
│   │   ├── extract.rs          # HAR parsing and configuration extraction
│   │   ├── validate.rs         # Imported-value validation and sanitization
│   │   ├── persist.rs          # Safe configuration-file persistence
│   │   └── handlers.rs         # Configuration HTTP responses and orchestration
│   ├── museai/
│   │   ├── mod.rs              # Narrow subsystem facade
│   │   ├── business.rs         # Pure provider rules
│   │   ├── handlers.rs         # HTTP adaptation only
│   │   ├── chat/
│   │   │   ├── mod.rs          # Chat facade
│   │   │   ├── service.rs      # Chat use-case orchestration
│   │   │   ├── parser.rs       # Incoming event interpretation
│   │   │   └── stream.rs       # Stream state and completion handling
│   │   ├── video.rs            # Video use case and artifact interpretation
│   │   ├── session.rs          # Bootstrap and credentials lifecycle
│   │   ├── threads.rs          # Thread tracking and cleanup lifecycle
│   │   ├── transport.rs        # Blocking WebSocket adapter
│   │   ├── noise.rs            # Noise session adapter
│   │   └── protocol.rs         # Wire encoding and decoding
│   ├── jev/
│   │   ├── mod.rs              # Narrow subsystem facade
│   │   ├── business.rs         # Pure provider rules
│   │   ├── handlers.rs         # HTTP adaptation only
│   │   ├── service.rs          # Provider request orchestration
│   │   ├── client.rs           # Blocking upstream I/O
│   │   └── pool.rs             # Account selection and rotation state
│   └── video_template.rs      # Existing UI facade and asset delivery
├── tests/                     # Detailed layout below
├── assets/                    # UI templates and static assets
├── docs/
│   ├── PROJECT_STRUCTURE.md   # This proposal
│   ├── SYSTEM_DESIGN.md       # Runtime behavior and subsystem ownership
│   ├── ARCHITECTURE.md        # Boundaries and design rationale
│   ├── SECURITY_MODEL.md
│   ├── CONFIGURATION.md
│   └── ...                    # Retain other existing documents
├── plan/
│   ├── pending/               # Unapproved or unverified work
│   └── done/                  # Completed and verified work
├── scripts/                   # Development and operational utilities
├── .github/                   # CI and repository automation
├── .claude/                   # Existing agent configuration/reference
└── target/                    # Generated artifacts; not source or fixtures
```

This is the intended ownership map, not a requirement to create empty files. Split a module only when it contains the named cohesive responsibility. Small cohesive files may remain intact. Target under 450 lines per module; files exceeding 800 lines require decomposition.

## Test layout

All test bodies and test-only helpers live physically under `tests/`. Small files directly under `tests/` are Cargo entry points, not monolithic suites.

```text
tests/
├── gateway.rs                  # Module declarations for the gateway suite
├── config.rs                   # Module declarations for configuration tests
├── muse.rs                     # Existing Muse integration target
├── jev.rs                      # Existing Jev integration target
├── video_template.rs           # Existing UI integration target
├── live.rs                     # Explicitly opted-in external checks
├── gateway/
│   ├── request_tests.rs
│   ├── response_tests.rs
│   ├── framing_tests.rs
│   ├── route_tests.rs
│   ├── fallback_tests.rs
│   └── rewrite_tests.rs
├── config/
│   ├── extract_tests.rs
│   ├── validation_tests.rs
│   ├── persistence_tests.rs
│   └── route_tests.rs
├── muse/
│   ├── chat_tests.rs
│   ├── video_tests.rs
│   ├── session_tests.rs
│   ├── threads_tests.rs
│   └── route_tests.rs
├── jev/
│   ├── route_tests.rs
│   ├── pool_tests.rs
│   └── failure_tests.rs
├── video_template/
│   └── video_template_tests.rs
├── unit/                       # Private-component tests, library test build only
│   ├── mod.rs                  # Test-only module registration
│   ├── muse/
│   │   ├── parser_tests.rs
│   │   ├── protocol_tests.rs
│   │   └── noise_tests.rs
│   └── ...                     # Other private algorithms only when needed
├── live/
│   ├── muse_auth_tests.rs
│   ├── muse_chat_tests.rs
│   ├── muse_video_tests.rs
│   └── muse_cleanup_tests.rs
├── support/
│   ├── mod.rs
│   ├── config.rs               # Test configuration builders
│   ├── mock_server.rs          # Loopback-only upstream doubles
│   ├── temp_files.rs           # Isolated temporary resources
│   └── live_session.rs         # External-test-only session support
└── fixtures/
    ├── http/
    ├── har/
    ├── muse/
    └── jev/
```

### Rust test discovery and privacy

- Cargo discovers top-level integration target files. Each target explicitly registers its nested test modules; simply adding a nested file does not register tests.
- `support/mod.rs` is explicitly included by targets that need it. Do not add a top-level `support.rs` that becomes an unintended Cargo target.
- Public behavior belongs in integration tests. Avoid reaching through the facade to internal provider modules.
- Files under `tests/unit/` are compiled through a `cfg(test)` library module, not a separate integration target. Registration must respect Rust privacy: attach private tests within the owning module when they need private members, or use existing crate-visible seams where appropriate. A root test module cannot automatically access all sibling-private items.
- A small test-module declaration may remain in `src/`; test bodies stay in `tests/`. Do not make implementation details public or duplicate production source just to access them in tests.
- `live.rs` needs an explicit opt-in mechanism, such as a required Cargo feature plus ignored live tests. Its folder name alone does not prevent default execution. Final gating is part of implementation approval.
- Default `cargo test` must not contact external services, even when credentials are present. Live tests document cost, prerequisites, resource ownership, and cleanup.
- Fixtures contain synthetic or sanitized data only. Do not copy raw files from `target/`.

## Dependency boundaries

1. The executable delegates startup to the library/runtime.
2. Runtime and routes orchestrate; they do not own provider business rules.
3. Provider HTTP handlers delegate to provider application logic.
4. Pure business rules do not depend on TCP, WebSocket, HTTP serialization, or persistence.
5. Transport/protocol code does not import HTTP route handlers.
6. Muse and Jev do not import each other's internals.
7. Production modules never import test support.
8. `mod.rs` files are facades and module declarations, not places to accumulate business logic. Keep internals private or crate-visible; preserve intentionally supported public entry points through compatible re-exports.

Keep synchronous I/O, fail-closed routing, and the existing single-crate build. A physical folder split is useful only when these responsibilities and dependencies are also clear.

## Mapping from the existing layout

| Existing location | Proposed ownership |
| --- | --- |
| Runtime currently in `src/lib.rs` | `src/runtime.rs`, retaining the library entry point |
| `src/http.rs` | `src/http/`, only if a cohesive split improves maintenance |
| `src/omniroute.rs` | `src/omniroute/`, separating selection, orchestration, and streaming |
| `src/har_config.rs` | `src/har_config/`, separating extraction, validation, persistence, HTTP |
| `src/museai/chat.rs` | `src/museai/chat/`, preserving its facade |
| `src/jev/handlers.rs` | HTTP adaptation stays; application and upstream work receive separate ownership |
| `src/test_helpers.rs` | `tests/support/`, without production test-support exports |
| `tests/gateway/core_http_tests.rs` | Focused gateway parsing/framing/response suites |
| `tests/gateway/protocol_omniroute_tests.rs` | Appropriate gateway suites and provider-owned protocol tests |
| `tests/gateway/har_config_tests.rs` | Configuration suite |
| `tests/gateway/muse_video_route_tests.rs` | Muse route/video ownership after inspecting each test |
| `tests/gateway/museai_live_tests.rs` | Offline parser/route checks separated from opt-in live checks |
| Existing `tests/muse/`, `tests/jev/`, `tests/video_template/` | Preserve target names; regroup only where responsibility warrants it |

These mappings describe destinations, not completed moves. Preserve uncommitted user work and compare test discovery before and after migration. Review compatibility before removing any currently public symbol.

## Approval and validation

Approve this folder design before migration. No directory skeleton, production edits, test moves, or Miro retries were performed for this clarification.

Design validation: inspected current `src/`, `tests/`, and `docs/` directories. This is documentation only; no build or test results are claimed.

After approved migration, inspect live-test gating, compare `cargo test -- --list` inventories, then run `cargo fmt`, `cargo clippy --all-targets`, `cargo test`, and `cargo build --release`. Live provider verification is separate and must not be inferred from mocks.
