# Security Policy

## Supported versions

The `main` branch is the only supported version. Fixes land there first and are
tagged afterwards.

## Reporting a vulnerability

Report privately via GitHub Security Advisories:

<https://github.com/giauphan/toolcase-gateway/security/advisories/new>

Please do not open a public issue for an exploitable defect. Include:

- Affected version or commit
- Reproduction steps or a proof-of-concept request
- Observed vs expected behavior
- Impact assessment

Expect an initial response within 7 days.

## Scope

In scope:

- HTTP parsing defects: request smuggling, header injection, response splitting
- Resource exhaustion: unbounded memory, thread, or file-descriptor growth
- Header leakage across the client/upstream boundary
- Response corruption caused by the tool-name rewriting logic

Out of scope:

- Lack of authentication on the listener. This is documented and intentional;
  the gateway is meant to run on loopback behind an authenticating front end.
- Vulnerabilities in the upstream LLM API itself.
- Denial of service that requires already-privileged local access.

## Hardening summary

- `#![forbid(unsafe_code)]`
- Zero third-party dependencies
- Strict HTTP framing validation (rejects conflicting `Content-Length` /
  `Transfer-Encoding`)
- Header name/value validation (tokens only; `CR`/`LF`/`NUL` refused)
- Bounded head (64 KB), body (64 MB), and rewrite window (1 MB)
- Bounded concurrency and per-socket I/O timeouts
- Hop-by-hop header stripping in both directions
- JSON-escaped error bodies

Details in [`docs/SECURITY_MODEL.md`](docs/SECURITY_MODEL.md).
