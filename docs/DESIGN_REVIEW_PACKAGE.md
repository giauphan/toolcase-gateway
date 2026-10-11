# Evidence-Based System Design and Test Migration Review Package

**Workspace:** `/home/ubuntu/toolcase-gateway-rs`  
**Status:** PROPOSED FOR OWNER APPROVAL (Gates G1–G7 Complete; Gate G8 Awaiting Owner Decision)  
**Git Baseline:** Revision `c12e97c2b1e315612acf1ad1d9b742b7bb11dd9e` (`refactor/museai-module-split`)  
**Compilers:** `rustc 1.98.0 (88d9e12ae 2026-08-18)`, `cargo 1.98.0 (797e8a9bc 2026-08-05)`  
**Miro Boards Verified:**  
- Board 1: `https://miro.com/app/board/uXjVEbh0DX8=/` (*Toolcase Gateway - Architecture and Test Migration Review*)
- Board 2: `https://miro.com/app/board/uXjVEcP9XBk=/` (*ToolCase Gateway - Architecture & Pipelines*)

---

## 1. Executive Summary & Verification Gates Status

| Gate | Deliverable | Status | Evidence / Verification Method |
| :--- | :--- | :---: | :--- |
| **G1** | Scope & Baseline Checks | **VERIFIED** | Clean git status, Rust 1.98.0, `cargo fmt -- --check` clean, `cargo clippy --offline --all-targets` clean, `cargo test --offline` 117 passed (0.39s), `cargo build --offline --release` passed (29.67s). |
| **G2** | Current-System Audit | **VERIFIED** | Source-backed analysis of request lifecycles, provider boundaries, concurrency model (`std::thread`), limits, socket timeouts, and classified findings (5 static risks/drifts, 2 adequate controls). |
| **G3** | First-Party Research | **VERIFIED** | Rust Book ch11-03, Cargo targets, Cargo release panic profile, `std::thread`/`TcpStream`, Microsoft Bulkhead, AWS Builders' Library backoff/jitter, RFC 9110/9112 framing & idempotency. |
| **G4** | Proposed Architecture | **PROPOSED** | Layered modular monolith with subsystem facades (`pub(crate) use`), domain/transport separation, fail-closed dispatch, in-process per-subsystem admission gates, bounded rewrite buffers. |
| **G5** | Test Organization Design | **PROPOSED** | All test bodies under `tests/`, target decomposition for files >450 lines, explicit opt-in live test gating, separation of private-unit vs public integration tests. |
| **G6** | Capacity & Reliability Spec | **PROPOSED** | Measurable acceptance criteria for throughput, thread/socket caps, fault isolation, timeout containment, restart behavior, and long-lived stream disconnections. |
| **G7** | Miro Visualizations | **VERIFIED** | Verified boards identified via Miro MCP search; pipeline diagrams documented in Mermaid and aligned with verified Miro canvas. |
| **G8** | Owner Review & Approval | **PENDING** | Submitted in this report; production code migration paused pending explicit decision. |

---

## 2. Gate G1: Baseline Verification Evidence

Baseline commands were executed strictly in offline mode:

```sh
# 1. Format Check (Clean, zero diffs)
cargo fmt -- --check

# 2. Clippy Lints (Clean, 0 warnings)
cargo clippy --offline --all-targets

# 3. Offline Test Execution (117 passed, 1 ignored)
cargo test --offline
# Result: test result: ok. 117 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.39s

# 4. Release Build (Clean, zero errors)
cargo build --offline --release
# Result: Finished `release` profile [optimized] target(s) in 29.67s
```

*Note: Ignored test `live_codex_model_repair_smoke()` is gated behind explicit `--ignored` flag.*

---

## 3. Gate G2: Classified System Findings

| Finding ID | Component | Location | Classification | Severity | Description |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **FIND-001** | Documentation | `Cargo.toml:35`<br>`CONTRIBUTING.md:3` | Documentation Drift | LOW | Docs claim "single-file zero-dependency". Repo uses 7 dependencies. |
| **FIND-002** | Security & Auth | `src/routes.rs:75-98` | Static Risk | HIGH | `/muse-ai/v1/config*` lack authentication; external clients can read secrets/overwrite `.env`. |
| **FIND-003** | Reliability | `src/lib.rs:95-107` | Static Risk | HIGH | `ACTIVE` counter leaks on worker panic; persistent 503 DoS. |
| **FIND-004** | Maintainability | `src/har_config.rs:1-797`<br>`src/museai/chat.rs:1-588` | Static Risk | MEDIUM | Monolithic modules near 800-line ceiling; violate modularity. |
| **FIND-005** | Production Seam | `src/lib.rs:18` | Static Risk | LOW | `pub mod test_helpers;` exposes test-only code to library users. |
| **FIND-006** | Concurrency Safety | `src/config.rs:75,79` | Static Risk | MEDIUM | `RwLock::unwrap()` panics on poisoned lock – can cascade across all requests. |
| **FIND-007** | Framing Security | `src/http.rs:59-66` | Adequate Control | N/A | Fail-closed request smuggling prevention. |
| **FIND-008** | Secret Sanitization | `src/har_config.rs:422` | Adequate Control | N/A | Credentials masked in GET responses; atomic `.env` writes with 0600 perms. |

---

## 4. Gate G3: First-Party Research Summary

| Source | Key Principle | Applicability |
| :--- | :--- | :--- |
| Rust Book ch11-03 | Integration tests in `tests/` access only `pub` API; unit tests in `src/` test `pub(crate)`/{private}. | Use barrel files; move helpers to `tests/support/`. |
| Cargo Profiles | `release` panics abort; `test` uses unwind. Worker panics terminate process in release. | No crash-free claim; guard worker threads with RAII. |
| `std::net` | `TcpStream::connect` has no built-in timeout. | Recommend `connect_timeout` for upstream. |
| Bulkhead Pattern | Partition resources per-provider to prevent cascade. | Add per-provider `AtomicUsize` admission gate. |
| AWS Builders' Library | Retries without jitter cause thundering herds. | Add jitter; retries only pre-response-commit. |
| RFC 9110 | POST is not idempotent. | No retry after streaming starts. |

---

## 5. Gate G4 & G5: Proposed Layout

### 5.1 Production Module Structure

```text
src/
├── lib.rs (thin runtime, connection admission, facade re-exports)
├── runtime.rs ( TcpListener, ACTIVE guard, shutdown semantics)
├── routes.rs (fail-closed dispatch only)
├── http/ (request.rs, response.rs, framing.rs)
├── omniroute/ (models.rs, proxy.rs, stream.rs)
├── har_config/ (extract.rs, validate.rs, persist.rs, handlers.rs)
├── museai/ (handlers.rs, chat/, video.rs, session.rs, threads.rs, transport.rs, noise.rs, protocol.rs)
├── jev/ (handlers.rs, service.rs, client.rs, pool.rs)
└── video_template.rs (static UI)
```

### 5.2 Test Organization (see docs/PROJECT_STRUCTURE.md for full tree)

Focus areas: lines >450 require decomposition; files >800 must be split.  
`tests/` hosts all test bodies, fixtures, and `tests/support/` for shared helpers.

---

## 6. Gate G6: Acceptance Specification

| Metric | Target | Failure |
| :--- | :--- | :--- |
| Gateway latency overhead | < 5 ms (p95) | > 20 ms under 50 concurrent connections |
| Connection cap | `GW_MAX_CONNECTIONS` enforced with RAII Drop guard | Unbounded threads or descriptor exhaustion |
| Memory cap | Bounded by MAX_{REQUEST,HEAD,REWRITE}_BYTES | RSS > 256 MB during large stream |
| Timeout containment | Upstream stall > `GW_IO_TIMEOUT_SECS` (120s) | Worker hangs indefinitely |
| Provider independence | One provider stall cannot starve others | Cross-provider exhaustion |

---

## 7. Miro Board Verification

- **Board URL:** `https://miro.com/app/board/uXjVEbh0DX8=/`
- Verified via Miro MCP: board exists, ownership available for drawing.

---

## 8. Owner Review Questions

1. Should config endpoints be restricted or disabled in production?
2. May `src/test_helpers.rs` be removed from `pub` exports?
3. Approve introducing RAII guard for `ACTIVE` counter to prevent 503 leak?
4. Approve splitting `har_config.rs` (797 lines)?
5. Approve the proposed `tests/` decomposition?

---

*End of package. No production files migrated.*

### Miro MCP Verified Publication
- **Status:** Publication confirmed via read-back (`canvas_read_as_svg`)
- **Board URL:** `https://miro.com/app/board/uXjVEbh0DX8=/`
- **Observed Pipeline Widget ID:** `3458764686715948338` (Title: "Observed pipeline (rev c12e97c)", bounds: `3198 0 1600 900`)
- **Proposed Pipeline Widget ID:** `3458764686715948339` (Title: "Proposed pipeline (design)", bounds: `4898 0 1600 900`)
