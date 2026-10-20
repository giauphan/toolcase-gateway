# Muse Video Generation

## 1. Overview

`toolcase-gateway` provides end-to-end video synthesis capabilities powered by Muse.ai Confidential VMs (CVM) and Noise protocol WebSockets at `/muse-ai/v1/create-video`.

Key features:
- **Durable Job Tracking**: Video jobs are recorded under `.gw_state/video_jobs/{job_id}.json` with status lifecycle and revision tracking.
- **Headless Sentinel Auto-Approval**: Evaluates permission requests automatically when configured with `GW_MUSE_APPROVAL_MODE=on` (or `scoped_auto`), preventing blocking on browser interaction.
- **Default Video Prompt Configuration**: Operators can define or update the default fallback generation prompt via the web dashboard at `/gateway-config/dashboard` or via `MUSE_LIVE_VIDEO_PROMPT`.
- **Idempotency Safeguards**: Repeated requests using the same `idempotency_key` or `submission_key` return existing job status without incurring duplicate generation costs.

---

## 2. API Endpoints

### 2.1 Create Video (`POST /muse-ai/v1/create-video`)

Submits a new text-to-video generation task.

#### Request Payload
```json
POST /muse-ai/v1/create-video
Content-Type: application/json

{
  "prompt": "A small orange fox wearing a blue scarf rides a skateboard through a colorful city park, bouncy animation, bright morning light.",
  "model": "muse-video",      // optional, defaults to "muse-video"
  "aspect_ratio": "16:9",      // optional, defaults to "16:9"
  "duration": 5,               // optional, defaults to 5 seconds
  "idempotency_key": "job-8814-fox" // optional
}
```

#### Response (`200 OK`)
```json
{
  "id": "video-123e4567-e89b-12d3-a456-426614174000",
  "object": "video.generation",
  "created": 1728000000,
  "provider": "muse_chat",
  "resolved_provider": "muse_chat",
  "requested_model": "muse-video",
  "resolved_model": "muse-video",
  "model": "muse-video",
  "prompt": "A small orange fox wearing a blue scarf rides a skateboard through a colorful city park...",
  "requested_aspect_ratio": "16:9",
  "requested_duration": 5,
  "status": "artifact_reported",
  "provider_status": "artifact_url_reported",
  "output_verified": false,
  "media_validated": false,
  "video_url": "https://cdn.muse.ai/files/v1.mp4"
}
```

---

### 2.2 Get Video Job Status (`GET /muse-ai/v1/videos/{id}`)

Retrieves status, artifacts, and approval details for a specific video job.

#### Request
```http
GET /muse-ai/v1/videos/video-123e4567-e89b-12d3-a456-426614174000
```

#### Response (`200 OK`)
```json
{
  "id": "video-123e4567-e89b-12d3-a456-426614174000",
  "object": "video.generation",
  "created": 1728000000,
  "provider": "muse_chat",
  "status": "artifact_reported",
  "approval_status": "approved",
  "approval_id": "app-42",
  "output_verified": false,
  "media_validated": false,
  "video_url": "https://cdn.muse.ai/files/v1.mp4",
  "prompt": "A small orange fox wearing a blue scarf rides a skateboard...",
  "model": "muse-video",
  "requested_model": "muse-video",
  "aspect_ratio": "16:9",
  "duration": 5,
  "thread_id": "thread-123e4567",
  "revisions": [],
  "last_activity_at": 1728000000
}
```

---

### 2.3 Update Video Prompt (`PUT /muse-ai/v1/videos/{id}`)

Updates the prompt of an existing video job and records a revision.

#### Request
```json
PUT /muse-ai/v1/videos/video-123e4567-e89b-12d3-a456-426614174000
Content-Type: application/json

{
  "prompt": "Updated prompt: enhance cinematic golden-hour lighting and dynamic camera angles."
}
```

#### Response (`200 OK`)
```json
{
  "id": "video-123e4567-e89b-12d3-a456-426614174000",
  "object": "video.generation",
  "updated_at": 1728000500,
  "prompt": "Updated prompt: enhance cinematic golden-hour lighting and dynamic camera angles.",
  "status": "updated"
}
```

---

### 2.4 Delete Thread (`DELETE /muse-ai/v1/threads/{id}`)

Cleans up an active thread on the Muse upstream server.

#### Request
```http
DELETE /muse-ai/v1/threads/thread-123e4567
```

#### Response (`200 OK`)
```json
{
  "ok": true,
  "thread_id": "thread-123e4567",
  "status": "deleted"
}
```

---

## 3. Headless Sentinel Auto-Approval & Idempotency

When enabled, the Rust-native Sentinel coordinator listens to approval events emitted during generation and resolves eligible, pre-authorized scoped approvals (e.g. `network`) automatically:

```mermaid
sequenceDiagram
    participant Gateway as toolcase-gateway
    participant CVM as Muse CVM (WebSocket)
    participant Sentinel as Sentinel Approval Broker

    Gateway->>CVM: Initialize generation prompt
    CVM-->>Gateway: Permission Prompt Event (e.g. network access)
    Gateway->>Gateway: Check Policy (scope, destination, type)
    alt Allowed by Policy
        Gateway->>Sentinel: POST /approvals/{id}/decide (allow_once)
        Sentinel-->>Gateway: HTTP 200 OK
        Gateway->>Sentinel: GET /approvals/{id} (verify status == approved)
        Sentinel-->>Gateway: Confirmed Approved
        CVM-->>Gateway: Stream resumes -> artifact URL reported
    else Rejected / Mode Off
        Gateway-->>Client: HTTP 409 Conflict (MuseApprovalRequired)
    end
```

### Configuration Parameters
- `GW_MUSE_APPROVAL_MODE`: `off` (default), `observe`, or `scoped_auto` / `on`.
- `GW_MUSE_APPROVAL_ALLOWED_DESTINATIONS`: Required comma-separated allowlist of destination hostnames (e.g. `muse.ai,googleapis.com`).
- `GW_MUSE_APPROVAL_ALLOWED_TYPES`: Comma-separated list of approved types (default: `network`). Connector approvals always require manual authorization.
- `GW_MUSE_APPROVAL_PREFER_ALLOW_ONCE`: `true` (default) or `false`.
- `GW_MUSE_APPROVAL_ALLOW_ALWAYS`: `false` (default) or `true`.
- `GW_MUSE_APPROVAL_MAX_DECISIONS`: Maximum decisions allowed per job (default: `3`).
- `GW_MUSE_APPROVAL_TIMEOUT_SECS`: Sentinel RPC timeout in seconds (default: `30`).

### Dynamic Policy Management via Dashboard
You can view and modify approval parameters in real time at `/gateway-config/dashboard` or via:
```bash
curl -X POST http://127.0.0.1:20129/api/v1/dashboard/muse-config \
  -H "Content-Type: application/json" \
  -d '{
    "approval_mode": "on",
    "allowed_destinations": ["muse.ai"],
    "allowed_types": ["network"],
    "default_video_prompt": "A majestic eagle soaring over snow-capped mountains"
  }'
```

### Dynamic Rollback
To immediately disable auto-approval without restarting the process:
1. Submit `POST /api/v1/dashboard/muse-config` with `{"approval_mode": "off"}`.
2. All pending approvals will immediately fail-closed and return HTTP 409 Conflict. Previous job history is preserved.

---

## 4. Supported Models & Aspect Ratios

### Supported Model Identifiers
- Canonical: `muse-video`
- Supported Aliases: `muse`, `gen-3`, `gen-2`, `kling`, `gen-4`, `gen-4.5`, `gen-4-turbo`, `aleph-2.0`, `ruby`

### Supported Aspect Ratios
- `16:9` (Default)
- `9:16`
- `1:1`
- `5:4`
- `4:3`

---

## 5. Error Handling

| Status Code | Error Code | Description |
| :--- | :--- | :--- |
| `400 Bad Request` | `INVALID_INPUT` | Missing prompt, empty string, or unsupported model/ratio. |
| `401 Unauthorized` | `AUTHENTICATION_REQUIRED` | Missing or invalid Muse credentials (`cookie`, `access_token`). |
| `409 Conflict` | `MuseApprovalRequired` | Pending approval required; auto-approval is off or destination is not allowlisted. |
| `501 Not Implemented` | `ARTIFACT_MISSING` | Generation finished without yielding a valid HTTPS video URL. |
| `502 Bad Gateway` | `UPSTREAM_TRANSPORT_ERROR` | Upstream WebSocket connection or Noise handshake failed. |
