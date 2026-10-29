# Muse Video Curl Guide and Smoke Test

This guide demonstrates how to create videos via `curl` against the local gateway (listening by default on port `20130`), wait for generation to complete, and directly receive playable video URLs.

> [!IMPORTANT]
> Ensure the gateway is running (`GW_LISTEN_HOST=0.0.0.0 GW_LISTEN_PORT=20130 ./target/release/toolcase-gateway`) and valid Muse credentials are configured via `/dashboard`, `gateway_config.json`, or `.env`.

---

## 1. Synchronous Video Creation (Wait for Completion)

To submit a video generation request that blocks until completion and directly returns the completed video link, pass `?wait=true` in the URL (or `"wait": true` in the JSON body):

```bash
curl --no-buffer -X POST \
  -H "Content-Type: application/json" \
  --data-raw '{
    "prompt": "A small cute golden retriever puppy sitting on green grass, 3d animation style",
    "model": "muse-video",
    "aspect_ratio": "16:9",
    "duration": 5
  }' \
  "http://127.0.0.1:20130/muse-ai/v1/create-video?wait=true"
```

### Expected Response (`200 OK`)

The HTTP connection stays open while the Confidential VM produces the video and the gateway streams the media over the encrypted Noise channel. When finished, it returns:

```json
{
  "id": "video-6d4a9d57-5c61-408b-8825-a32047b83596",
  "object": "video.generation",
  "created": 1792640786,
  "status": "completed",
  "output_verified": true,
  "media_validated": true,
  "video_url": "/muse-ai/v1/videos/video-6d4a9d57-5c61-408b-8825-a32047b83596/content",
  "full_video_url": "http://127.0.0.1:20130/muse-ai/v1/videos/video-6d4a9d57-5c61-408b-8825-a32047b83596/content",
  "model": "muse-video",
  "prompt": "A small cute golden retriever puppy sitting on green grass, 3d animation style",
  "requested_aspect_ratio": "16:9",
  "requested_duration": 5
}
```

---

## 2. Verifying and Streaming the Video Media

Once the video is generated, you can inspect or stream the real MP4 media:

### Check Video Headers (`HEAD` or `curl -I`)

```bash
curl -I http://127.0.0.1:20130/muse-ai/v1/videos/video-6d4a9d57-5c61-408b-8825-a32047b83596/content
```

Response:
```http
HTTP/1.1 200 OK
Content-Type: video/mp4
Content-Length: 5925734
Accept-Ranges: bytes
Access-Control-Allow-Origin: *
Content-Disposition: inline; filename="video-6d4a9d57-5c61-408b-8825-a32047b83596.mp4"
```

### Download the Video to Disk

```bash
curl -o output.mp4 http://127.0.0.1:20130/muse-ai/v1/videos/video-6d4a9d57-5c61-408b-8825-a32047b83596/content
```

### Play in the Browser or Video Player

Open the player in your browser at:
`http://127.0.0.1:20130/video-template?job_id=video-6d4a9d57-5c61-408b-8825-a32047b83596`

---

## 3. Querying Video Job Details

You can query the durable job record anytime:

```bash
curl -s http://127.0.0.1:20130/muse-ai/v1/videos/video-6d4a9d57-5c61-408b-8825-a32047b83596
```

---

## 4. Asynchronous Submission (Optional)

If you prefer non-blocking submission and polling:

```bash
curl -X POST "http://127.0.0.1:20130/muse-ai/v1/create-video?async=true" \
  -H "Content-Type: application/json" \
  -d '{"prompt": "A sunset over the mountains", "model": "muse-video"}'
```

Returns immediately with:
```json
{
  "id": "video-...",
  "status": "submitting",
  "video_url": ""
}
```

Then poll status until completed:
```bash
curl -s http://127.0.0.1:20130/muse-ai/v1/videos/video-...
```

---

## 5. Handling Approvals (`409 Conflict`)

If the response returns `409 Conflict` with `MuseApprovalRequired`:
1. **Manual Resolution**: Open the session in your browser and grant permission. Then rerun the curl command with the same prompt.
2. **Headless Auto-Approval**: Enable headless Sentinel auto-approvals in the dashboard at `http://127.0.0.1:20130/dashboard` or via:
   ```bash
   curl -X POST http://127.0.0.1:20130/api/v1/dashboard/muse-config \
     -H "Content-Type: application/json" \
     -d '{"approval_mode": "on", "allowed_destinations": ["muse.ai"], "allowed_types": ["network"]}'
   ```
