# Contributing

Thanks for helping out. This project is deliberately small: one file, no
dependencies. Keep it that way where you can.

## Setup

Requires Rust 1.74 or newer.

```sh
git clone https://github.com/giauphan/toolcase-gateway.git
cd toolcase-gateway
cargo build
```

## Before opening a PR

```sh
cargo fmt
cargo clippy --all-targets
cargo test
cargo build --release
```

All four must pass.

## Ground rules

- **No new dependencies.** Zero-dependency is a feature, not an accident. If you
  believe a crate is unavoidable, open an issue first and explain why std cannot
  do it.
- **No `unsafe`.** `#![forbid(unsafe_code)]` is enforced.
- **Add a test for behavior changes.** Parsing and rewriting bugs are easy to
  introduce and cheap to pin down with a unit test.
- **Security-relevant changes need a doc update.** If you touch framing
  validation, limits, or header handling, update `docs/SECURITY_MODEL.md`.
- **Keep the diff focused.** Do not reformat unrelated code or rename things in
  passing.

## Commit messages

- Imperative mood, capitalized, no trailing period
- Subject under 50 characters
- Body wrapped at 72 characters, only when it adds information

```
Reject duplicate Content-Length headers

Two Content-Length values let a downstream proxy and this gateway
disagree on body length, which is the basis of a smuggling attack.
```

## Reporting bugs

Include the gateway version or commit, the relevant environment variables, the
stderr output, and a minimal request that reproduces the problem. Redact API
keys.

For anything exploitable, use the private advisory flow in
[`SECURITY.md`](SECURITY.md) instead of a public issue.
