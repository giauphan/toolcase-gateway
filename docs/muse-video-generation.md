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

### Cleanup
- Thread retention is controlled by `GW_MUSEAI_THREAD_RETENTION_SECS` (default: 86400 seconds = 1 day, or 259200 for 3 days).
- Video job cleanup is handled by the background worker in `src/providers/museai/lifecycle.rs`.

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
