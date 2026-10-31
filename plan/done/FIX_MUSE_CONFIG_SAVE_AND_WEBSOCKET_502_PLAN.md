# Fix Muse Config Save and WebSocket 502 Plan

## Goal

Ensure Muse configuration updates submitted via the Dashboard UI and API persist accurately to memory and persistent storage without being reset to placeholder defaults, and resolve the Muse WebSocket connection failure (`HTTP error: 502 Bad Gateway`) during image generation by correctly handling VM identifier resolution, session credentials, and endpoint fallback behavior.

## Input

- `src/features/dashboard/muse.rs`: Handler for `/api/v1/dashboard/muse-config` and payload parsing/validation.
- `src/features/dashboard/overview.rs`: Endpoint `/api/v1/dashboard/overview` providing configuration data to the dashboard frontend.
- `assets/ui/dashboard.js`: Frontend logic for populating the Muse form and submitting settings.
- `assets/dashboard.html`: Muse form elements, input names, IDs, and placeholder attributes.
- `src/features/har/handler.rs`: HAR import handler `/muse-ai/v1/config/har` updating active runtime settings and file storage.
- `src/config/persistent.rs` & `src/config/store.rs`: Persistent configuration model and disk synchronization.
- `src/modules/builtins/museai/session/mod.rs` & `src/modules/builtins/museai/session/bootstrap.rs`: WebSocket URL construction, session metadata parsing, and VM address resolution.
- `src/modules/builtins/museai/transport.rs`: WebSocket client connection and error handling.
- `src/modules/builtins/museai/image/service.rs`: Image generation request pipeline and completion dispatch.
- `gateway_config.json`: Persistent gateway configuration on disk.
- Test suites in `tests/dashboard.rs`, `tests/video_template/`, and `tests/har_config/`.

## Output

### Quality

- Muse configuration updates submitted via `/api/v1/dashboard/muse-config` reliably update both in-memory configuration (`ConfigStore`) and persistent storage (`gateway_config.json`).
- Payload parsing accepts both nested `approval: { ... }` objects and flat approval fields seamlessly without data loss.
- The dashboard frontend accurately reads effective Muse configuration from overview responses and preserves user-entered values rather than reverting to hardcoded placeholder defaults (`vm-438`, `wss://vm-438.metaaivm.com/`).
- Placeholder attributes in HTML describe format hints rather than functioning as artificial fallbacks for live traffic.
- HAR configuration ingestion persists effective values to both `.env` and `gateway_config.json`.
- Muse WebSocket URL and VM ID construction prioritizes valid live session metadata and configured VM identifiers over stale or dummy values.
- Upstream WebSocket error handling provides explicit diagnostics distinguishing invalid VM targets, authentication expiration (401/403), and gateway/routing failures (502).
- All source files conform to Clean Architecture file size limits (strictly under 450 lines per module).
- Maintain zero-async invariant (`std::net`, `std::thread`, no tokio).

### Test

- Unit and integration tests in `tests/dashboard.rs` covering `/api/v1/dashboard/muse-config` with both nested `approval` and flat approval payloads.
- Tests verifying that saved values are persisted to disk and accurately reflected in subsequent `/api/v1/dashboard/overview` requests.
- Tests verifying that HAR ingestion updates runtime configuration and persistent file storage.
- Tests in `tests/video_template/` and session modules verifying URL construction and session metadata application with real and custom VM IDs.
- Full test suite passes without regressions.

### Validate

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- Verification of JavaScript syntax and cognitive complexity via `node -c assets/ui/dashboard.js`.

### Output rules

- Do not commit or print actual cookies, tokens, API keys, or private URLs.
- Maintain zero-async architecture (`std::net`, `std::thread` only).
- Keep changes modular, maintaining backward compatibility of all existing routes.

### Verify

- Execute curl requests against local gateway instances testing Muse config update, overview query, and persistence verification.
- Test image generation endpoint behavior and error formatting on invalid/expired VM states.

## Research pointers

- Meta CVM Noise protocol handshake and endpoint routing documentation.
- `src/features/dashboard/muse.rs` payload definition vs frontend serialization in `assets/ui/dashboard.js`.
- `src/modules/builtins/museai/session/mod.rs` and `bootstrap.rs` for VM ID resolution and session lifecycle.
- `src/config/persistent.rs` for field mappings between runtime and persistent configuration.
