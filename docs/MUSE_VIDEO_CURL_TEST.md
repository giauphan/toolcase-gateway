# Muse Video Curl Smoke Test

Use this smoke test when the gateway is running and valid Muse credentials are configured (via `/gateway-config/dashboard`, `gateway_config.json`, or `GW_MUSEAI_COOKIE`).

> [!IMPORTANT]
> Keep the JSON payload on one physical line. Newlines inside the `prompt` string produce invalid JSON.

## 1. Video Generation Test

```bash
curl --no-buffer --max-time 930 -X POST \
  -H "Content-Type: application/json" \
  --data-raw '{"prompt":"Create a cheerful cartoon-style short video: a small orange fox wearing a blue scarf rides a skateboard through a colorful city park, bouncy animation, bright morning light, playful camera moves, whimsical clouds, no text, no logos, family-friendly.","model":"muse-video","aspect_ratio":"16:9","duration":5}' \
  http://127.0.0.1:20129/muse-ai/v1/create-video
```

### Expected Response (`200 OK`)
A successful response returns `status: "artifact_reported"` (or `"completed"`) and a valid `video_url`:
```json
{
  "id": "video-123e4567-e89b-12d3-a456-426614174000",
  "object": "video.generation",
  "status": "artifact_reported",
  "video_url": "https://cdn.muse.ai/files/v1.mp4",
  ...
}
```

---

## 2. Handling Approvals (`409 Conflict`)

If the response returns `409 Conflict` with `MuseApprovalRequired`:
1. **Manual Resolution**: Open the session in your browser and grant permission. Then rerun the curl command with the same prompt.
2. **Headless Resolution**: Enable headless auto-approvals in the dashboard at `http://127.0.0.1:20129/gateway-config/dashboard` or via:
   ```bash
   curl -X POST http://127.0.0.1:20129/api/v1/dashboard/muse-config \
     -H "Content-Type: application/json" \
     -d '{"approval_mode": "on", "allowed_destinations": ["muse.ai"], "allowed_types": ["network"]}'
   ```
