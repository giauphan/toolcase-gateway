# Muse AI Conversational Chat

## 1. Overview

`toolcase-gateway` provides an OpenAI-compatible conversational chat interface powered by **Muse.ai Confidential VMs (CVM)**. Chat requests are encrypted using the Noise protocol, routed directly to isolated remote VMs, and streamed back as standard OpenAI chat completions.

Key capabilities:
- **Dedicated & Global Endpoints**: Dispatched directly via `POST /muse-ai/v1/chat/completions` or globally via `POST /v1/chat/completions` with `"model": "muse"`.
- **OpenAI Drop-In Compatibility**: Supports standard `messages` arrays, role attribution, and returns standard `chat.completion` objects.
- **Real-Time Web Search & Live Retrieval**: The Muse CVM LLM can execute real-time searches and tool calls to answer live queries (e.g., financial spot prices, current events, macroeconomic data).
- **Headless Sentinel Auto-Approval**: Network and browsing actions requested by the remote VM can be auto-approved using the gateway's Sentinel approval coordinator (`GW_MUSE_APPROVAL_MODE=on`).
- **End-to-End Cryptographic Isolation**: Client prompts are encrypted through a 3-way mutual `Noise_XX_25519_AESGCM_SHA256` handshake directly to the Confidential VM.
- **Thread Tracking & Lifecycle Retention**: Every conversation is assigned an isolated `thread_id` / `session_id` and automatically purged after a configurable retention period.

---

## 2. API Endpoints

| Endpoint | Method | Purpose | Description |
| :--- | :--- | :--- | :--- |
| `/muse-ai/v1/chat/completions` | `POST` | Dedicated Chat Completions | Canonical endpoint dedicated to Muse AI chat. |
| `/muse-ai/chat/completions` | `POST` | Dedicated Route Alias | Convenience path without the `/v1` segment. |
| `/v1/chat/completions` | `POST` | Global Proxy Completions | OmniRoute proxy route (requires `"model": "muse"`). |
| `/muse-ai/v1/chat/completions` | `OPTIONS`| CORS Preflight | Returns `200 OK` with standard CORS headers. |
| `/muse-ai/v1/models` | `GET` | Model Catalog | Returns catalog entry for model `muse`. |
| `/muse-ai/v1/threads/:id` | `DELETE` | Thread Cleanup | Manually terminates and cleans up a remote thread. |

> [!NOTE]
> Non-POST HTTP methods on `/muse-ai/v1/chat/completions` (such as `GET`, `PUT`, or `DELETE`) fail closed with **`405 Method Not Allowed`**.

---

## 3. Quickstart Examples

### 3.1 Basic Chat Request (Dedicated Route)

```bash
curl -X POST http://127.0.0.1:20130/muse-ai/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "muse",
    "messages": [
      {
        "role": "user",
        "content": "Hello! Explain what a Confidential VM is in 2 sentences."
      }
    ]
  }'
```

### 3.2 Live Search & Financial Analysis Query

The Muse CVM can look up live web sources and technical indicators to answer real-time questions:

```bash
curl -X POST http://127.0.0.1:20130/muse-ai/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "muse",
    "messages": [
      {
        "role": "user",
        "content": "let'\''s search price of XAU/USD today and it will how to 2 hours next"
      }
    ]
  }'
```

### 3.3 Global OpenAI Proxy Route

If your client or SDK points to a generic OpenAI base URL (e.g. `http://127.0.0.1:20130/v1`):

```bash
curl -X POST http://127.0.0.1:20130/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "muse",
    "messages": [
      {
        "role": "user",
        "content": "Provide a brief summary of quantum computing."
      }
    ]
  }'
```

---

## 4. Request & Response Schemas

### 4.1 Request Format

Requests conform to the standard OpenAI chat completions schema.

```json
{
  "model": "muse",
  "messages": [
    {
      "role": "system",
      "content": "You are a helpful financial analyst."
    },
    {
      "role": "user",
      "content": "What is the spot price of gold today?"
    }
  ]
}
```

#### Fields

| Field | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `model` | `string` | Optional* | Target model. Recommended value is `"muse"`. When calling `/muse-ai/v1/chat/completions`, defaults to `muse`. |
| `messages` | `array` | Optional* | Array of message objects containing `role` (`"user"`, `"assistant"`, `"system"`) and `content`. |
| `prompt` | `string` | Optional* | Alternative top-level string prompt if not using the `messages` array. |

*\*At least one of `messages` or `prompt` must be provided.*

### 4.2 Response Format

Completions return a standard OpenAI `chat.completion` object containing metadata, token counts, and session tracking identifiers:

```json
{
  "id": "chatcmpl-5b14f739-e4a2-4dcc-9ca6-7e2ab27c3bc1",
  "object": "chat.completion",
  "created": 1728000000,
  "model": "muse",
  "thread_id": "54515aa9-0d4e-44dd-bbb1-3bc61363509a",
  "session_id": "54515aa9-0d4e-44dd-bbb1-3bc61363509a",
  "choices": [
    {
      "index": 0,
      "message": {
        "role": "assistant",
        "content": "The spot price of gold (XAU/USD) today is trading around $4,062 - $4,089 per ounce..."
      },
      "finish_reason": "stop"
    }
  ],
  "usage": {
    "prompt_tokens": 12,
    "completion_tokens": 145,
    "total_tokens": 157
  }
}
```

#### Extension Fields
- `thread_id`: Unique identifier for the conversation thread assigned by the Muse CVM.
- `session_id`: Unique session identifier used for stream subscription and approval scoping.

---

## 5. Live Search & Real-Time Data

When asked queries requiring current real-world information (such as commodity prices, foreign exchange rates, or news events), the Muse Confidential VM can initiate search tools:

1. **Tool Invocation**: The CVM LLM recognizes that fresh data is required and triggers an internal search or browsing tool.
2. **Sentinel Security Check**: If the external lookup requires egress authorization, the remote daemon emits an `approvals.snapshot` event.
3. **Auto-Approval**: The gateway's Sentinel coordinator evaluates the domain and action type. If permitted by policy, the action is automatically approved and execution continues seamlessly.
4. **Data Aggregation**: The CVM ingests the live results and synthesizes a comprehensive response with current numbers and market context.

---

## 6. Sentinel Auto-Approval Configuration

When the remote CVM performs actions that require permission, it issues a structured approval snapshot. By default, the gateway operates in `scoped_auto` mode (or `GW_MUSE_APPROVAL_MODE=on`), allowing approved domains to execute without blocking the user:

### Environment Variables

```ini
# Approval mode: on (scoped_auto), observe, or off
GW_MUSE_APPROVAL_MODE=on

# Comma-separated list of permitted network destinations
GW_MUSE_APPROVAL_ALLOWED_DESTINATIONS=muse.ai,api.muse.ai

# Permitted action types
GW_MUSE_APPROVAL_ALLOWED_TYPES=network

# Automatically prefer single-use permission grants
GW_MUSE_APPROVAL_PREFER_ALLOW_ONCE=true

# Maximum auto-approvals permitted per chat session
GW_MUSE_APPROVAL_MAX_DECISIONS=5
```

### Approval Statuses
- **Approved**: Action destination matches `allowed_destinations` and type matches `allowed_types`. Gateway issues an instant approval token.
- **Conflict (`409 Conflict`)**: Action requires human review (e.g. connector integration or untrusted domain). The gateway rejects the completion request with HTTP 409, prompting the user to grant permission in the browser.

---

## 7. Cryptographic Transport Architecture

Conversational requests are handled entirely in memory with strict cryptographic guarantees:

```mermaid
sequenceDiagram
    autonumber
    participant Client as Client Application
    participant Gateway as toolcase-gateway
    participant CVM as Muse Confidential VM (hatch.metaaivm.com)

    Client->>Gateway: POST /muse-ai/v1/chat/completions (JSON)
    Gateway->>CVM: WebSocket Connect (wss://hatch.metaaivm.com/v1/noise)
    Gateway->>CVM: Noise XX 3-Way Handshake (Curve25519, AES-GCM)
    Gateway->>CVM: Encrypted Service Request: POST /chat/stream
    CVM-->>Gateway: Encrypted Stream: delta.text_append / task.complete
    opt If CVM Requests External Action
        CVM-->>Gateway: approvals.snapshot
        Gateway->>CVM: Auto-Approve (if destination allowlisted)
    end
    Gateway->>Client: HTTP 200 OK (OpenAI chat.completion JSON)
```

1. **VM Wake & Notary Token**: If not statically supplied, the gateway fetches notary and access tokens from `POST /api/hatch/token`.
2. **Noise XX Mutual Handshake**: Ephemeral Curve25519 keypairs are generated per connection; payloads are encrypted with `AES-25519-GCM` with SHA-256 hashing.
3. **Stream Subscription**: The client subscribes to Stream 2 (`/chat/subscribe`) to receive streaming text chunks (`delta.text_append`).
4. **Assembly**: Slices are parsed by `MuseChatStream` and formatted into a unified `chat.completion` response.

---

## 8. Thread Lifecycle & Cleanup

Every conversation thread generates a remote thread resource inside the CVM. To prevent unmanaged resource accumulation, `toolcase-gateway` provides an automated background thread lifecycle daemon:

```ini
# Enable automatic cleanup of chat threads
GW_MUSEAI_AUTO_CLEANUP_THREADS=true

# Retention period before thread deletion (seconds; default: 172800 = 2 days)
# Set to 0 for immediate deletion upon completion response
GW_MUSEAI_THREAD_RETENTION_SECS=172800
```

- When `GW_MUSEAI_THREAD_RETENTION_SECS=0`, threads are deleted immediately after the assistant finishes responding.
- When retention is greater than 0, threads are tracked in the gateway's thread registry and purged by the background maintenance daemon on 300-second intervals once their TTL expires.
- Manual cleanup is always available via `DELETE /muse-ai/v1/threads/:id`.

---

## 9. Error Reference & Troubleshooting

| HTTP Status | Error Type | Cause | Resolution |
| :--- | :--- | :--- | :--- |
| `400 Bad Request` | `Invalid JSON` or empty prompt | Request payload was not valid JSON, or both `messages` and `prompt` were missing. | Verify that request body contains valid JSON with a `messages` array or `prompt` string. |
| `401 Unauthorized` | `PermissionDenied` | Muse authentication cookie or access token is missing, expired, or invalid. | Update `GW_MUSEAI_COOKIE` or upload a fresh HAR capture via `/muse-config`. |
| `405 Method Not Allowed` | `Method Not Allowed` | Calling `/muse-ai/v1/chat/completions` with `GET`, `PUT`, or `DELETE`. | Use `POST` for chat completions. |
| `409 Conflict` | `MuseApprovalRequired` | The CVM requested an action (e.g. connector access) that is not in the Sentinel auto-approval allowlist. | Open the Muse web UI in your browser to approve the pending action, or add the destination to `GW_MUSE_APPROVAL_ALLOWED_DESTINATIONS`. |
| `502 Bad Gateway` | `Bad Gateway` | Remote CVM WebSocket reset, network handshake failed, or upstream server error. | Verify internet connectivity to `metaaivm.com` and ensure VM is in `RUNNING` state. |
| `504 Gateway Timeout` | `Gateway Timeout` | Remote CVM did not return completion tokens within the operation deadline (default: 360s). | Check remote service status and network latency. |
