# Configuration

All configuration is environment variables, read once at startup. There is no
config file and no command-line flags. Restart to apply changes.

## Reference

| Variable | Default | Type | Description |
| --- | --- | --- | --- |
| `GW_LISTEN_HOST` | `127.0.0.1` | host | Bind address. Non-loopback values log a warning at startup. |
| `GW_LISTEN_PORT` | `20129` | u16 | Listen port. |
| `GW_TARGET_HOST` | `127.0.0.1` | host | Upstream host to proxy to. |
| `GW_TARGET_PORT` | `20128` | u16 | Upstream port. |
| `GW_FALLBACK_MODELS` | `fail-try` | csv | Fallback models tried in round-robin order after the requested model. |
| `GW_MAX_CONNECTIONS` | `256` | usize | Concurrent connection cap. Excess connections get `503`. |
| `GW_IO_TIMEOUT_SECS` | `120` | u64 | Per-socket read and write timeout. `0` disables timeouts. |

Empty or whitespace-only values fall back to the default. Unparseable numeric
values fall back to the default rather than failing startup.

## Compile-time limits

These are constants in `src/main.rs`; change them and rebuild if your workload
needs different bounds.

| Constant | Value | Purpose |
| --- | --- | --- |
| `MAX_HEAD_BYTES` | 64 KB | Cap on request and response head size. |
| `MAX_REQUEST_BYTES` | 64 MB | Cap on request body size. |
| `MAX_REWRITE_WINDOW` | 1 MB | Forced flush point for newline-free response bodies. |
| `RETRYABLE` | `402, 403, 408, 429, 500, 502, 503, 504, 524` | Statuses that trigger failover. |

## Failover behavior

The attempt order is the model from the request body first, then each fallback.
Fallbacks rotate: request *n* starts at `fallbacks[n % len]`, so repeated
failures spread across secondaries instead of hammering one.

```sh
GW_FALLBACK_MODELS="gpt-4o-mini,claude-3-5-sonnet,llama-3.3-70b"
```

A request naming `gpt-4o` produces the chain
`gpt-4o -> gpt-4o-mini -> claude-3-5-sonnet -> llama-3.3-70b` on the first
request, and `gpt-4o -> claude-3-5-sonnet -> llama-3.3-70b -> gpt-4o-mini` on the
next. Duplicates are removed, so naming a model that is also a fallback does not
retry it twice.

To disable failover entirely, set the variable to a single space:

```sh
GW_FALLBACK_MODELS=" "
```

With no fallbacks the requested model is the only candidate and its response is
returned as-is, including retryable statuses.

## Tuning

**Long streaming completions.** A large reasoning response can idle between
tokens. If you see connections dropped mid-stream, raise the timeout:

```sh
GW_IO_TIMEOUT_SECS=600
```

**Many parallel agents.** Each in-flight request holds one thread and two
sockets. Raise the cap only alongside the process file-descriptor limit:

```sh
GW_MAX_CONNECTIONS=512
```

**Low-memory hosts.** Worst-case memory is roughly
`GW_MAX_CONNECTIONS * (request body + rewrite window)`. Lowering
`MAX_REQUEST_BYTES` and `MAX_REWRITE_WINDOW` in source is the effective lever.

## Deployment recipes

### Loopback beside a local router (recommended)

```sh
GW_TARGET_PORT=8080 ./target/release/toolcase-gateway
```

Client points at `http://127.0.0.1:20129`.

### systemd user service

```ini
[Unit]
Description=toolcase-gateway
After=network.target

[Service]
ExecStart=%h/.local/bin/toolcase-gateway
Environment=GW_TARGET_PORT=8080
Environment=GW_FALLBACK_MODELS=gpt-4o-mini,claude-3-5-sonnet
Environment=GW_IO_TIMEOUT_SECS=300
Restart=on-failure
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=strict
ProtectHome=read-only

[Install]
WantedBy=default.target
```

### Container

```dockerfile
FROM rust:1-slim AS build
WORKDIR /src
COPY . .
RUN cargo build --release

FROM gcr.io/distroless/cc
COPY --from=build /src/target/release/toolcase-gateway /toolcase-gateway
ENV GW_LISTEN_HOST=0.0.0.0
EXPOSE 20129
ENTRYPOINT ["/toolcase-gateway"]
```

Setting `GW_LISTEN_HOST=0.0.0.0` is required inside a container, but the gateway
has no authentication. Publish the port only to a trusted network, or place an
authenticating reverse proxy in front. See [`SECURITY_MODEL.md`](SECURITY_MODEL.md).

## Logging

Diagnostics go to stderr, one line each, no configurable level:

```
[toolcase-gateway] 127.0.0.1:20129 -> 127.0.0.1:20128
[toolcase-gateway] upstream model "gpt-4o" returned HTTP 429. Failing over to "gpt-4o-mini"...
[toolcase-gateway] upstream model "gpt-4o" failed (ConnectionRefused). Failing over to "gpt-4o-mini"...
[toolcase-gateway] request failed: InvalidData
```

Request and response bodies are never logged, so prompts and API keys stay out of
the log stream. Expected disconnects (`BrokenPipe`, `ConnectionReset`,
`UnexpectedEof`) are silent.
