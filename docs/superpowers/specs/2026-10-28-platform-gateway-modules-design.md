# Platform Gateway Modules Design

## Purpose

Turn the gateway into a modular platform where every provider is an explicit,
runtime-enabled module with a unique public namespace.  OmniRoute becomes a
module served only at `/ominroute/...`; it is no longer the fallback target for
unowned URLs.

## Scope and decisions

- A module is a statically compiled integration with runtime enablement.  This
  change does not introduce dynamic native-code or script loading.
- The canonical OmniRoute namespace is `/ominroute/...` (the requested
  spelling).  Its upstream request path is the portion after that prefix.
- Muse and Jev remain provider modules and retain their established public
  namespaces as compatibility contracts: `/muse-ai/...` and `/jev/...`.
- Dashboard, configuration, templates, and other gateway-owned pages are
  system endpoints, not provider modules.
- The former implicit proxy route is removed.  URLs that do not belong to a
  system endpoint or an enabled module fail locally without upstream traffic.
- Existing provider protocol, authentication, retry, streaming, and lifecycle
  behavior remains provider-owned.  The migration changes ownership and
  admission, not those provider semantics.

## Architecture

The platform has four dependency layers:

1. **Gateway kernel** defines provider-neutral module identity, namespace
   ownership, and enablement policy.  It has no HTTP, socket, credential, or
   concrete-provider dependency.
2. **Configuration/runtime** stores validated module enablement alongside the
   current immutable request snapshot and wires built-in modules at process
   startup.
3. **Application/provider modules** implement a module's use cases and own
   provider-specific payloads, credentials, account state, and upstream
   protocols.  They may use gateway-kernel policy but do not own public URL
   matching.
4. **Inbound HTTP adapters** normalize a request path, enforce method and
   namespace boundaries, ask the kernel for its owner, and delegate to the
   appropriate use case.  They own HTTP errors and response commitment only.

The dependency direction is inward: HTTP adapters depend on application and
kernel contracts; provider modules depend on their application contracts and
the kernel; the kernel depends on neither.  Runtime composition is the only
place permitted to bind a concrete provider to a module contract.

## Module registry and lifecycle

The registry is an authoritative, finite catalog of shipped modules.  For each
module it provides:

- a stable identifier for configuration and observability;
- its canonical namespace boundary;
- whether it is enabled in the current validated configuration; and
- enough route classification information for the inbound adapter to reject
  disabled, incomplete, and lookalike namespaces before provider execution.

The registry does not carry provider secrets, live sockets, or provider client
instances.  It is deterministic and testable from paths plus configuration.
Runtime enablement is persisted using the existing protected configuration
store and exposed by the existing administration surface with no secret values.
Built-in modules are enabled by default to maintain availability after upgrade,
but OmniRoute's old unprefixed access is intentionally not retained because it
violates the requested ownership rule.

## Routing behavior

The inbound adapter first handles gateway-owned system endpoints.  It then
classifies the remaining normalized path against module boundaries.

- An enabled module receives only a path below its exact namespace.
- A disabled but known module returns a local unavailable/not-found gateway
  error according to the existing route error convention, without upstream
  contact.
- An unknown, incomplete, malformed, or lookalike module namespace returns a
  local `404`.
- A recognized module with an unsupported method returns local `405`.
- The OmniRoute adapter receives the route beneath `/ominroute`; for example,
  `/ominroute/v1/chat/completions` maps to its existing OpenAI-compatible
  proxy operation for `/v1/chat/completions`.
- No path falls through to a generic upstream proxy.

The existing large dispatcher is split by ownership into small inbound-adapter
modules.  Shared path normalization and module admission live at the adapter /
kernel boundary, while each provider's existing request semantics stay in its
provider/application module.

## Configuration and administration

Module enablement is a typed, validated part of the configuration snapshot and
persistent representation.  Configuration changes use the current atomic file
replacement and owner-only permissions.  Dashboard data presents module state
and accepts only known module identifiers; invalid names or malformed state are
rejected before persistence.  Neither the dashboard nor registry responses
include provider credentials.

## Compatibility and error handling

Muse and Jev retain their documented provider paths while being admitted by the
same module policy.  Existing internal façade re-exports remain until callers
are migrated; facades do not bypass registry admission.  Configuration created
before this migration acquires default built-in module enablement when loaded.

All existing framing checks, request-size admission, CORS handling, response
stream commitment, secret masking, and upstream retry rules continue to apply.
Once an upstream response has been committed, no routing or module failover is
attempted.

## Testing and verification

The migration is test-first.  Unit tests cover pure module identity,
enablement, and path classification.  Route contract tests cover every
shipped namespace, disabled modules, path-boundary lookalikes, method errors,
and rejection of former unowned paths.  Integration tests use a synthetic
upstream to prove that `/ominroute/v1/chat/completions` forwards the stripped
path and that `/v1/chat/completions` produces no upstream request.

Existing OmniRoute retry/candidate/stream tests and existing Muse/Jev contract
tests remain required.  Configuration persistence tests prove enablement
survives reload without exposing secrets.  Final validation runs `cargo fmt
--check`, `cargo clippy --all-targets`, `cargo test`, and `cargo build
--release`, plus the synthetic-upstream smoke check.

## Non-goals

- Runtime installation of arbitrary third-party executable plugins.
- A new asynchronous runtime or dependency framework.
- Changing provider wire protocols, credentials, retry classifications, or
  account-pool rules unless directly required by module admission.
- Retaining the generic OmniRoute catch-all as a hidden compatibility path.
