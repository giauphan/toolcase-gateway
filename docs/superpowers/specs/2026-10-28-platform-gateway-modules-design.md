# Platform Gateway Modules and MCP Control Plane Design

## Purpose

Turn the gateway into an extensible modular platform where:
1. Every provider is an explicit, runtime-enabled module with a unique public
   namespace (OmniRoute is served only at `/ominroute/...` and is no longer an
   implicit fallback).
2. Users can author and install personal provider modules without needing the
   gateway codebase to exist or recompiling the gateway binary.
3. AI agents can connect to the gateway via an official Model Context Protocol
   (MCP) control plane to discover installed modules, inspect configurations,
   and safely invoke authorized management tools.

## Scope and Decisions

- **Built-in provider modules**: Statically compiled integrations (OmniRoute,
  Muse, Jev) with dynamic runtime enablement.
- **Personal / out-of-tree provider modules**: Authored independently in
  external repositories with no dependency on gateway source code.  Installed
  and managed dynamically via declarative manifests and out-of-process
  sandboxing (subprocess stdio/IPC or local HTTP/socket adapter) to preserve
  zero-async and `#![forbid(unsafe_code)]`.
- **MCP control-plane interface**: Implements the official Model Context
  Protocol specification using JSON-RPC 2.0 over standard transports (`stdio`
  and Streamable HTTP/SSE), exposing typed tools and resources for gateway
  administration.
- **Namespaces**:
  - OmniRoute: canonical namespace `/ominroute/...` (upstream path is the suffix).
  - Muse: `/muse-ai/...`.
  - Jev: `/jev/...`.
  - Personal modules: Unique declared namespace (e.g. `/personal/<name>/...` or
    validated custom root prefix), validated against collisions with system
    routes and built-in modules.
- **Fail-closed routing**: Implicit fallbacks are removed.  Requests to
  unregistered, disabled, or malformed namespaces fail locally without upstream
  traffic.
- **Security & Secret Redaction**: All MCP tools, resources, logs, and dashboard
  surfaces redact or omit API keys, tokens, and session cookies.

## Architecture & Dependency Layers

Enforcing Clean Architecture and Domain-Driven Design (DDD) with inward-only
dependencies:

```
+-----------------------------------------------------------------------+
|                         Interface Adapters                            |
|  +--------------------+  +---------------------+  +----------------+  |
|  | Inbound HTTP (8080)|  | MCP Server (stdio)  |  | MCP (HTTP/SSE) |  |
|  +--------------------+  +---------------------+  +----------------+  |
|  +-----------------------------------------------------------------+  |
|  |            Out-of-Process Adapter (Subprocess / IPC)            |  |
|  +-----------------------------------------------------------------+  |
+-----------------------------------------------------------------------+
                                  │ (calls inward)
                                  ▼
+-----------------------------------------------------------------------+
|                      Application Services & Use Cases                 |
|  - Module Registry Service (Built-in + Personal Modules)               |
|  - Module Lifecycle Manager (Install, Enable, Disable, Remove)        |
|  - MCP Tool Dispatcher & Access Controller                             |
|  - Request Orchestrator & Proxy Service                               |
+-----------------------------------------------------------------------+
                                  │ (calls inward)
                                  ▼
+-----------------------------------------------------------------------+
|                            Domain Kernel                              |
|  - Module Identity, Manifest, and Capability Models                   |
|  - Namespace Ownership and Collision Policy                           |
|  - Model Candidate Selection, Retry Classification, and Error Domain  |
+-----------------------------------------------------------------------+
```

1. **Domain Kernel**: Defines provider-neutral entities (`ModuleId`,
   `Namespace`, `ModuleManifest`, `Capability`, `PolicyDecision`).  Has zero
   dependencies on HTTP, JSON-RPC, sockets, subprocesses, or credentials.
2. **Application Services**: Coordinates use cases (`RegisterModule`,
   `EnableModule`, `ExecuteProviderRequest`, `ExecuteMcpTool`).  Enforces
   authorization and configuration snapshots.
3. **Interface Adapters**:
   - `InboundHttpAdapter`: Parses HTTP requests, matches registered namespaces,
     and translates responses.
   - `McpAdapter`: Parses JSON-RPC 2.0 framing over `stdio` or HTTP/SSE,
     handles standard MCP lifecycle handshakes, and maps MCP tool calls to
     application services.
   - `ExternalProcessAdapter`: Spawns and manages external module worker
     subprocesses, serializing request/response frames over standard IPC
     without unsafe native linking.
4. **Configuration & Persistence**: Atomically persists module enablement and
   registered personal module manifests in `gateway_config.json`.

## Personal / Out-of-Tree Module System

To allow external developers to write and install modules without the gateway
codebase:

1. **Manifest Contract (`module.json` / `manifest.json`)**:
   - `id`: Unique identifier (e.g., `"custom-cohere-adapter"`).
   - `name`: Human-readable display name.
   - `version`: SemVer string.
   - `namespace`: Canonical public URL prefix (e.g., `"/custom/cohere"`).
   - `entrypoint`: Executable command or script to spawn.
   - `protocol`: Communication protocol (e.g., `"stdio-stream"` or `"local-http"`).
   - `capabilities`: Array of supported capabilities (`["chat_completions", "models"]`).
   - `config_schema`: JSON schema of required/optional runtime settings.
2. **Isolation & Sandboxing**:
   - External modules run out-of-process.
   - The gateway manages child process lifecycle or communicates over loopback/stdio.
   - Fault containment: If an external module panics or exits, the gateway
     returns `502 Bad Gateway` locally; the core gateway remains unaffected.
3. **Lifecycle Management**:
   - Modules can be installed, validated, enabled, disabled, and removed via
     the MCP control plane, CLI commands, or dashboard.
   - Duplicate namespaces or attempts to shadow `/ominroute`, `/muse-ai`, or
     system endpoints are rejected on registration.

## Model Context Protocol (MCP) Control Plane

The MCP server acts as an administrative control plane for AI agents:

### Protocol Conformance
- Implements JSON-RPC 2.0 framing and standard MCP schemas.
- Supports two official transports:
  - `stdio`: Newline-delimited JSON-RPC over stdin/stdout. `stdout` is strictly
    reserved for protocol messages; application logs are directed to `stderr`.
  - `Streamable HTTP` / `SSE`: An HTTP endpoint (e.g., `/mcp`) supporting POST
    for JSON-RPC requests and GET for SSE streaming notifications.
- Protocol Lifecycle:
  - `initialize`: Returns server details (`toolcase-gateway`), protocol version,
    and advertised capabilities (`tools`, `resources`).
  - `notifications/initialized`: Handshake completion.
  - `ping`: Standard keepalive response.

### Exposed MCP Tools
1. `gateway_list_modules`:
   - Returns all registered modules (built-in and personal), namespaces,
     enablement status, and declared capabilities.
2. `gateway_get_module_info`:
   - Returns detailed manifest, runtime health, and sanitized configuration for
     a given module ID.
3. `gateway_set_module_state`:
   - Enables or disables a specified module dynamically.
4. `gateway_install_personal_module`:
   - Validates and registers an out-of-tree personal module from a package path
     or manifest definition.
5. `gateway_remove_personal_module`:
   - Unregisters and cleans up an installed personal module.
6. `gateway_update_provider_config`:
   - Safely updates non-secret provider options or candidate models.
7. `gateway_get_system_metrics`:
   - Returns sanitized system statistics (uptime, active connections, request
     counts, error rates).

### Exposed MCP Resources
- `gateway://modules`: Read-only list of installed modules and namespaces.
- `gateway://routes`: Read-only map of active route bindings.
- `gateway://health`: Read-only view of gateway admission and subsystem health.

### Authorization & Safety
- Management tools enforce capability permissions.
- Secret Sanitization: Under no circumstances do MCP tool results, resources,
  or errors contain unmasked API tokens, private keys, or passwords.

## Routing Behavior

The inbound HTTP adapter routes incoming requests through the following steps:
1. **System Routes**: Gateway dashboard, assets, and health checks.
2. **MCP Control Plane**: `/mcp` endpoints when HTTP/SSE transport is enabled.
3. **Module Namespaces**:
   - Normalized request path is matched against active module boundaries in the
     registry.
   - Built-in modules: `/ominroute/...`, `/muse-ai/...`, `/jev/...`.
   - Personal modules: Matched against their declared and validated namespaces.
   - The stripped sub-path is forwarded to the corresponding provider adapter.
4. **Fail-Closed Fallback**:
   - Disabled module -> Local `503 Service Unavailable` or `404 Not Found`.
   - Unknown/unregistered path -> Local `404 Not Found`.
   - Unsupported HTTP method -> Local `405 Method Not Allowed`.
   - No path ever falls through to an unowned generic upstream proxy.

## Configuration & Persistence

- Module enablement and personal module registry metadata are stored in
  `gateway_config.json`.
- Changes made via MCP tools or the dashboard trigger atomic file replacement
  with owner-only permissions (`0600`).
- Credentials remain encrypted or stored in secure environment stores; only
  redacted representations are returned via management interfaces.

## Testing & Verification

1. **Module Registry Unit Tests**:
   - Verification of built-in and external module registration.
   - Namespace collision detection and path prefix validation.
2. **MCP Protocol Conformance Tests**:
   - JSON-RPC 2.0 message envelope parsing and formatting.
   - Lifecycle handshake (`initialize`, `initialized`, `ping`).
   - Standard error handling (-32700, -32600, -32601, -32602, -32603).
   - `tools/list` and `tools/call` execution and validation.
   - Verification of credential masking in all MCP outputs.
3. **Personal Module Isolation Tests**:
   - Subprocess spawning, communication, and crash containment.
   - Rejection of malicious or malformed manifests.
4. **Route Contract & Regression Tests**:
   - OmniRoute prefixing at `/ominroute/...` with synthetic upstream.
   - Fail-closed rejection of former unprefixed proxy routes.
   - Muse and Jev backwards compatibility.
5. **Quality Gates**:
   - `cargo fmt --check`
   - `cargo clippy --all-targets`
   - `cargo test`
   - `cargo build --release`

## Non-Goals

- Loading untrusted shared libraries (`.so` / `.dylib`) in-process via `unsafe`.
- Introducing third-party asynchronous runtimes (`tokio`, `async-std`).
- Allowing arbitrary shell execution from unvalidated manifests.
- Exposing secrets or credentials over MCP tools or resources.
