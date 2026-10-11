# Migration Evidence Ledger: V4 System Design Compliance

**Document Status:** Complete & Verified
**Date:** 2026-10-11
**Target:** Architecture V4 (`plan/migration-system-design/ARCHITECTURE_V4.md`)

---

## 1. Toolchain & Runtime Baseline (PR0)
- **Rust Compiler & Cargo:** `rustc 1.98.0`, `cargo 1.98.0` (MSRV declared: `"1.74"`)
- **Compilation Safety:** `#![forbid(unsafe_code)]` enforced and verified.
- **Concurrency & I/O:** Synchronous blocking model (`std::net::TcpListener`, `std::thread`). Zero async runtime (`tokio`/`async-std`).

## 2. Gate Status Ledger (PR0 – PR7)
| Gate | Stage | Status | Verified Implementation & Evidence |
|---|---|---|---|
| **PR0** | Baseline & Fact Reconciliation | **Passed** | Inventoried Cargo manifest, dependencies, test suite structure, and module layouts. Placed canonical documentation under `docs/architecture/v4/`. |
| **PR1** | Route & Security Boundaries | **Passed** | Added `tests/routes_contract.rs` and validated comprehensive integration test suite. Added `route_namespace_fail_closed_behavior_offline` testing 405/404 fail-closed route behavior. |
| **PR2** | Centralized Process State | **Passed** | Promoted `JevAccountPool` to long-lived process singleton initialized once in `ConfigStore::new` (`src/config.rs`) and shared across request threads via `src/providers/jev/`. |
| **PR3** | Pure Gateway Decisions | **Passed** | Extracted pure candidate planning (`src/gateway/candidates.rs`) and retry safety (`src/gateway/retry.rs`, forbidding blind retries of 400 Bad Request) from network I/O. Added `status_400_does_not_retry_or_fail_over` and `retry_classification_handles_429_and_5xx_correctly` tests. |
| **PR4** | Transparent vs. Adapted Modes | **Passed** | Verified transparent proxying alongside adapter-based streaming and tool-name case repair. |
| **PR5** | Codecs & Contracts | **Passed** | Implemented capability-neutral Server-Sent Events decoding codec (`src/codecs/sse.rs`) supporting LF/CRLF/CR delimiters, multiline framing, and unit tests. |
| **PR6** | Integration Ownership | **Passed** | Decomposed OmniRoute into application feature proxy (`src/features/proxy/service.rs`) and provider adapter (`src/providers/omniroute/adapter.rs`), Jev into `src/providers/jev/`, and MuseAI into `src/providers/museai/` under target modular V4 directory structure (`src/features/`, `src/providers/`), with facade re-exports in `src/omniroute.rs`. |
| **PR7** | Reliability & Limits | **Passed** | Implemented RAII connection and byte quotas (`src/runtime/admission.rs`), wired admission control into connection accept loop in `src/lib.rs`, enforced `MAX_STREAM_BYTES` stream bounds in `src/http.rs`, and verified via `admission_controller_quota_guards_and_release`. |

## 3. Validation Summary
- `cargo fmt --all` (passed)
- `cargo clippy --all-targets` (passed)
- `cargo test --all-targets` (passed all 138 tests across gateway, jev, muse, video template, codecs, gateway, and runtime suites)
- `cargo build --release` (passed with 0 errors and 0 warnings)
