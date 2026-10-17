# Muse Video Generation

## Overview
The toolcase-gateway supports Muse video generation through `/muse-ai/v1/create-video`.

## API Endpoints

### Create Video (`POST /muse-ai/v1/create-video`)

Creates a new video generation job.

#### Request
```json
POST /muse-ai/v1/create-video
Content-Type: application/json

{
  "model": "gen-3",          // optional, defaults to "muse-video"
  "prompt": "A cat wearing sunglasses riding a skateboard in Tokyo", // required
  "aspect_ratio": "16:9",    // optional, defaults to "16:9"
  "duration": 5              // optional, defaults to 5 seconds
}
```

#### Response (Success - 200 OK)
```json
{
  "id": "video-123e4567-e89b-12d3-a456-426614174000",
  "object": "video.generation",
  "created": 1728000000,
  "provider": "muse_chat",
  "resolved_provider": "muse_chat",
  "requested_model": "gen-3",
  "resolved_model": "gen-3",
  "model": "gen-3",
  "prompt": "A cat wearing sunglasses riding a skateboard in Tokyo",
  "requested_aspect_ratio": "16:9",
  "requested_duration": 5,
  "status": "artifact_reported",
  "provider_status": "artifact_url_reported",
  "output_verified": false,
  "media_validated": false,
  "video_url": "https://muse.ai/files/v1.mp4"
}
```

---

### Get Video Job Status (`GET /muse-ai/v1/videos/{id}`)

Retrieves the current status and metadata of a video generation job.

#### Request
```http
GET /muse-ai/v1/videos/video-123e4567-e89b-12d3-a456-426614174000
```

#### Response (Success - 200 OK)
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
  "video_url": "https://muse.ai/files/v1.mp4",
  "prompt": "A cat wearing sunglasses riding a skateboard in Tokyo",
  "model": "gen-3",
  "requested_model": "gen-3",
  "aspect_ratio": "16:9",
  "duration": 5,
  "thread_id": "thread-123e4567",
  "revisions": [],
  "last_activity_at": 1728000000
}
```

---

### Update Video Prompt (`PUT /muse-ai/v1/videos/{id}`)

Updates the generation prompt for an existing video job.

#### Request
```json
PUT /muse-ai/v1/videos/video-123e4567-e89b-12d3-a456-426614174000
Content-Type: application/json

{
  "prompt": "Updated video generation prompt with enhanced cinematic details"
}
```

#### Response (Success - 200 OK)
```json
{
  "id": "video-123e4567-e89b-12d3-a456-426614174000",
  "object": "video.generation",
  "updated_at": 1728000500,
  "prompt": "Updated video generation prompt with enhanced cinematic details",
  "status": "updated"
}
```

## Video Job Lifecycle

### Job Persistence
- Every generated video is stored as `VideoJobRecord` in `.gw_state/video_jobs/{job_id}.json`.
- The record contains job metadata, model settings, timestamps, persistence state, and revision history.

### Cleanup & Retention
- Upstream Muse thread retention is controlled by `GW_MUSEAI_THREAD_RETENTION_SECS` (default: 86400 seconds = 1 day; can be set to 259200 for 3 days). A background thread worker periodically cleans up expired threads.
- Video job metadata records are persisted under `.gw_state/video_jobs/` and can be evaluated for expiration using `evaluate_job_retention` / `cleanup_expired_video_jobs`. Active or pinned jobs are never deleted.

### Headless Sentinel Auto-Approval & Idempotency

When enabled, the Rust-native Sentinel client listens to approval snapshots emitted during chat streams and automatically decides eligible, operator-preauthorized scoped approvals (e.g., `network`) on the same thread without opening a browser or repeating prompt submission.

#### Configuration Variables
- `GW_MUSE_APPROVAL_MODE`: `off` (default), `observe`, or `scoped_auto`; `on` is an alias for `scoped_auto`.
- `GW_MUSE_APPROVAL_ALLOWED_TYPES`: Comma-separated list of approved types (default: `network`). Connector approvals require manual authorization even when listed; Google Drive permission changes can grant public access.
- `GW_MUSE_APPROVAL_ALLOWED_DESTINATIONS`: Required comma-separated allowlist of destination hostnames for automatic approval; empty default approves nothing.
- `GW_MUSE_APPROVAL_PREFER_ALLOW_ONCE`: `true` (default) or `false`.
- `GW_MUSE_APPROVAL_ALLOW_ALWAYS`: `false` (default) or `true` (requires affirmative opt-in).
- `GW_MUSE_APPROVAL_MAX_DECISIONS`: Maximum decisions allowed per job (default: `3`).
- `GW_MUSE_APPROVAL_TIMEOUT_SECS`: Sentinel RPC timeout in seconds (default: `30`).

#### Idempotency
Clients can provide `idempotency_key` (or `submission_key`) in the JSON payload of `POST /muse-ai/v1/create-video`. Repeated submissions with the same key return the existing job record without re-invoking upstream generation.

#### Rollback
To rollback or disable auto-approval immediately:
1. Set `GW_MUSE_APPROVAL_MODE=off` in `.env`.
2. Restart the gateway.
All pending approvals will fail closed with HTTP 409 Conflict, preserving previous job history.

### Supported Models
```rust
const SUPPORTED_MODELS: [&str; 10] = [
    "muse", "muse-video", "gen-3", "gen-2", "kling",
    "gen-4", "gen-4.5", "gen-4-turbo", "aleph-2.0", "ruby"
];
```

## Error Responses

```json
{
  "error": {
    "code": "INVALID_INPUT",
    "category": "invalid_input",
    "retryable": false,
    "submission_unknown": false,
    "message": "missing 'prompt' field"
  }
}
```

## Testing

- **Route Tests**: `tests/gateway/muse_video_route_tests.rs`
- **Lifecycle & Persistence Tests**: `tests/muse/lifecycle_tests.rs`

All 176 tests pass.
