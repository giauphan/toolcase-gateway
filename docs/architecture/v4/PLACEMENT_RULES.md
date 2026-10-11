# Toolcase Gateway V4 — Code Placement and Import Rules

**Read before creating a Rust module.** The most important question is *who owns the behavior?*, not *is this shared?*

## Code-placement algorithm

1. Is it parsing/serializing inbound HTTP or enforcing privileged client access? → `api/`.
2. Is it a user-visible operation (transparent proxy, chat, models, video)? → `features/<capability>/`.
3. Is it a provider's credentials, payload shape, API workflow, WebSocket or account behavior? → `providers/<provider>/`.
4. Is it a pure policy shared between features (routing, retry eligibility, attempt classification)? → `gateway/`.
5. Is it an external wire format genuinely used by multiple provider/feature modules? → `codecs/`.
6. Is it outbound networking, secret hygiene, logging or metrics? → `infrastructure/`.
7. Is it process lifetime, config generation, shared registry or resource admission? → `runtime/`.
8. Is it a 5–20 line generic helper? → private beside first user; move only when semantics align and tests prove sharing.
9. If still uncertain, keep it private near the call site and write a short comment explaining responsibility; **do not add** `common.rs`, `utils.rs`, `helpers.rs`, `base_service.rs`, `service_manager.rs`, or `generic_provider.rs` to hide ambiguity.

## Examples

| Scenario | Destination | Anti-pattern |
|---|---|---|
| Add Muse video artifact URL validation | `providers/museai/video.rs` | global `utils/url.rs` |
| Add one new HTTP header validation rule | `api/http1/framing.rs` | `gateway/validators.rs` |
| Add capability-based candidate filter | `gateway/candidates.rs` | each provider's `routing.rs` |
| Add Jev-specific credit exhausted status classification | `providers/jev/adapter.rs` | generic HTTP `status_to_retry()` |
| Add general HTTP retry budget logic | `gateway/retry.rs` | copy to `providers/{x}/retry.rs` |
| Add SSE frame parser for Muse and OmniRoute | `codecs/sse.rs` | `features/chat/utils.rs` |
| Add tool-name casing reconciliation | `features/chat/tool_names.rs` | `codecs/sse.rs` |
| Add cookie-safe debugging | `infrastructure/secrets.rs` | raw `Debug` derive on credential |
| Add request-scoped config snapshot | `runtime/config.rs` | `env::var` inside `features/chat/service.rs` |
| Add model alias specific to Prism | `providers/prism/adapter.rs` | `gateway/candidates.rs` |
| Add generic proxy fallback route | `features/proxy/service.rs` | a catch-all in every provider handler |

## Review checklist

- Which feature owns it? Can another module change this without editing the owner?
- Is the new public API smaller than the implementation detail exposed?
- Is this a real capability contract rather than an interface with one implementation and no testing seam?
- Is there any dependency edge from `gateway`/`features` to a specific provider? That is a design smell.
- Is a helper being moved to shared solely because two strings look similar? Demand shared semantics, not duplicated syntax.
- Is it safe to retry? Distinguish replayability from HTTP status and downstream commitment.
- Did you leave an old wrapper/facade for existing imports during the move?
- Is the test placed with its owner and is the integration contract covered in `tests/`?
- Are sensitive tokens, cookies and raw HAR captures absent from fixtures, errors, and logs?

## Rust-specific rules

- Keep `main.rs` thin; `lib.rs` owns module declarations and testable exports.
- Default to private modules/functions; use `pub(super)`/`pub(crate)` before `pub`.
- Do not box every type or put everything behind a trait. Dynamic dispatch is reasonable at interchangeable provider capability boundaries.
- Avoid `Arc<Mutex<GatewayEverything>>`. Prefer immutable request snapshots and scoped state handles.
- Keep blocking network operations outside locks. Represent deadlines explicitly.
- No async runtime switch as part of unrelated folder reorganization.
- Follow repository AGENTS.md limits: target <=450 lines, soft ceiling <=800 lines, but split by semantic cohesion, not arbitrary line slicing.
- Keep the current Cargo MSRV/edition unless intentionally changed with CI/compiler validation.
