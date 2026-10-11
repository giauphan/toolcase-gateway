# Toolcase Gateway V4 — Safe Migration Sequence

**Not an implementation plan.** This is an architecture sequencing reference. The project's `AGENTS.md` requires an actual feature/change plan in `plan/pending/` before any code edit. Preserve that separate plan-writing contract.

## PR0 — Baseline and reconcile facts (no behavior changes)

- Checkout latest `main`; inventory `src` files, imports, outgoing network libraries, APIs, tests and feature gates.
- Map each actual file to a target owner. Resolve discrepancies between docs and Cargo manifest.
- Snapshot wire-level response/route behavior with synthetic secrets and mock upstream fixtures.
- Capture compiler version, MSRV compatibility and current test results.
- Done when a reviewer can explain route → handler → provider → response for chat, generic proxy, Jev failover and Muse video.

## PR1 — Prove route/security boundaries

- Add method/path regression matrix for `/prism-openai`, `/jev`, `/jev-ai`, `/muse-ai`, generic proxy and admin endpoints.
- Confirm privileged write APIs fail closed and aren't affected by permissive generic proxy behavior.
- Preserve public contract; security regressions are fixed separately and explicitly.

## PR2 — Centralize process state without moving providers

- Construct Jev pool once; use a shared lifetime and clear rotation/cooldown invariants.
- Wrap validated runtime config and request snapshots; add unit tests for reload success/failure and redaction.
- Isolate global admission accounting from specific handlers.

## PR3 — Extract pure gateway decisions

- Split candidate planning, error classification, retry budgets and commit checks from upstream network calls.
- Use table-driven tests for 400/401/403/429/500/503 and unknown-delivery cases; do not globally retry all client errors.
- Preserve current ordering behavior until a policy migration is explicitly approved.

## PR4 — Add transparent-vs-adapted boundary

- Introduce explicit routing outcomes: transparent proxy, typed chat/models/video, protected admin, 404, 405.
- Make generic passthrough stay compatible with unknown upstream payload fields.
- Put chat-specific tool-name correction behind its feature and preserve bounded stream behavior.

## PR5 — Extract codecs and provider contracts

- Build fixtures for SSE LF/CRLF and multi-line events. Decoding and semantic rewriting must be separately testable.
- Define capability-specific narrow traits only for genuinely interchangeable feature seams.
- Keep Muse's Noise/WebSocket protocol private; do not extract it to a global codec folder.

## PR6 — Modularize integrations gradually

- Refactor a small provider first (e.g. Jev) via facade-compatible moves.
- Decompose OmniRoute orchestration into provider adapter vs global proxy and routing policy.
- Move Muse files only in focused PRs where they demonstrably need changed ownership.
- Do not change routes and replace the HTTP runtime in the same PR.

## PR7 — Reliability and measured scaling

- Add in-flight-byte and connection budget guards, deadlines, cancellation and slow-client tests.
- Measure concurrent long-lived streams, p50/p95/p99, heap allocation and partial-disconnect behavior.
- Decide blocking I/O vs Tokio based on results and operational goals; do not rewrite based on fashion.

## Exit criteria

- `cargo fmt --all -- --check`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `cargo build --release`
- Mock upstream contract suites, stream golden fixtures, security tests.
- Docs/AGENTS aligned on a single canonical architecture and linked in README.
- Report behavioral changes, limitations and rollback details per PR.

## Mapping to check against real checkout

| Existing (documented, subject to HEAD confirmation) | Proposed owner |
|---|---|
| `src/routes.rs` | `src/api/router.rs` |
| `src/http.rs` | `src/api/http1/` |
| `src/rewrite.rs` | `src/features/chat/tool_names.rs` + limited proxy rewrite / codec |
| `src/omniroute.rs` | `src/features/proxy/service.rs` + `src/providers/omniroute/adapter.rs` + `src/gateway/{candidates,retry}.rs` |
| `src/jev/pool.rs` | `src/providers/jev/accounts.rs` (long-lived handle constructed by runtime) |
| `src/jev/handlers.rs` | thin `api/` inbound + Jev adapter |
| `src/museai/*` | keep as-is initially; potentially `src/providers/museai/*` later |
| `src/har_config.rs` | `src/api/admin/har.rs` + secure runtime config mutation |
| `src/config.rs` | `src/runtime/config.rs` |
| `src/tests.rs` | local `#[cfg(test)]` modules and `tests/` contract/fixture suites |

**Do not create a PR moving every file to the target layout.** Make boundaries testable first, rename after responsibilities are separated.
