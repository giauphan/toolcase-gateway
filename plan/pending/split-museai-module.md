# Split Muse integration module

## Goal

Decompose the monolithic `src/museai.rs` module into focused, cohesive modules to reduce maintenance friction, respect repository line-count maintainability guidelines, and improve isolation between distinct runtime flows while keeping external and internal behavior identical.

## Input

- `src/museai.rs` — monolithic integration file covering WebSocket setup, config bootstrap, chat streams, video generation, thread lifecycle, and HTTP handlers.
- `src/museai_business.rs`, `src/museai_noise.rs`, `src/museai_protocol.rs`, `src/museai_transport.rs` — existing sibling modules whose interfaces and boundaries must be preserved.
- `src/main.rs`, `src/routes.rs`, `src/omniroute.rs` — internal consumers relying on crate-level Muse functions.
- `src/tests.rs` and inline unit tests — deterministic verification baseline.
- `docs/MODELS.md` and `docs/SECURITY_MODEL.md` — public contract and security boundary specifications.
- `CONTRIBUTING.md` and `AGENTS.md` — quality and development constraints.

## Output

### Quality

- The codebase maintainability is improved by keeping modules focused around cohesive feature areas.
- No public, route-level, or OmniRoute-facing behavior changes.
- Crate-private entry points (`crate::museai::*`) remain backward-compatible for existing callers.
- Security-critical invariants remain preserved:
  - Allowed WebSocket authority checks for `*.metaaivm.com`.
  - Credential masking and safe error propagation.
  - Fail-fast handling of scoped native approval requirements (`MuseApprovalRequired`).
  - Strict HTTPS artifact URL validation for public Google Drive and direct video media.
  - Bounded socket operations and presentation waiting windows.
- No circular dependencies between sibling modules.

### Test

- Existing offline unit and integration tests remain passing without modification or weakening.
- Regression tests continue to cover:
  - Session bootstrap and WebSocket URL construction.
  - Multi-frame stream decoding, status snapshots, and assistant completions.
  - Direct and nested presentation video artifact extraction.
  - Explicit video refusal fast-failing.
  - Missing-artifact error mapping to HTTP 501.
  - Thread registration, retention, and cleanup.
- Test coverage across all extracted units must meet or exceed the existing baseline.

### Validate

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
git diff --check
```

### Output rules

- Zero new third-party dependencies.
- No `unsafe` code (`#![forbid(unsafe_code)]` remains strictly enforced).
- Do not modify or leak sensitive credentials, tokens, or raw captures.
- Keep `.claude/settings.json`, private marker files, and intermediate test artifacts untracked.

### Verify

- Run the full static check and offline test suite before and after the refactoring.
- Ensure all crate-level APIs used by `src/main.rs`, `src/routes.rs`, and `src/omniroute.rs` compile cleanly without churn in consumer modules.
- Bounded read-only live checks remain opt-in and may be executed only when authorized.

## Research pointers

- Examine existing sibling modules (`src/museai_transport.rs`, `src/museai_noise.rs`) for established conventions in visibility and error handling.
- Review `docs/SECURITY_MODEL.md` for trust boundaries governing Muse connections and live capture handling.
- Inspect `src/routes.rs` and `src/omniroute.rs` to verify exact call boundaries.
