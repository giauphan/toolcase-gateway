# Muse Video Curl Guide and Smoke Test

This guide demonstrates how to create videos via `curl` against the local gateway (listening by default on port `28130`), wait for generation to complete, and directly receive playable video URLs.

> [!IMPORTANT]
> Ensure the gateway is running (`GW_LISTEN_HOST=0.0.0.0 GW_LISTEN_PORT=28130 ./target/release/toolcase-gateway`) and valid Muse credentials are configured via `/dashboard`, `gateway_config.json`, or `.env`.

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
  "http://127.0.0.1:28130/muse-ai/v1/create-video?wait=true"
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
  "full_video_url": "http://127.0.0.1:28130/muse-ai/v1/videos/video-6d4a9d57-5c61-408b-8825-a32047b83596/content",
  "model": "muse-video",
  "prompt": "A small cute golden retriever puppy sitting on green grass, 3d animation style",
  "requested_aspect_ratio": "16:9",
  "requested_duration": 5
}
```

---

## 2. Image-to-Video Conditioning (Multipart Image Attachments)

To animate an existing picture (Image-to-Video), supply the reference image via multipart form upload (`-F "image=@..."` or `-F "images[]=@..."`) or via JSON data URI / internal reference.

> [!NOTE]
> The gateway automatically resolves the image, uploads it directly into the remote CVM disk at `workspace/your_files/` using 512 KB chunked streaming over Noise XX, and instructs Muse AI using native `file_ref` wire attachments. This completely avoids upstream LLM refusals such as *"Your image came through inline, not as a file I can use... If you attach the image as a file with the paperclip, I can animate that exact image"*.

### 2.1 Single Image Attachment via Multipart
```bash
curl -X POST http://127.0.0.1:20129/muse-ai/v1/create-video \
  -F "prompt=Animate this owl gently turning its head and blinking its glowing eyes, keeping the moonlit radiant wings intact." \
  -F "model=muse-video" \
  -F "aspect_ratio=16:9" \
  -F "duration=10" \
  -F "image=@/path/to/owl.png"
```

### 2.2 Multiple Images Attachment via Multipart
```bash
curl -X POST http://127.0.0.1:20129/muse-ai/v1/create-video \
  -F "prompt=Create a smooth cinematic morph transition between these two character poses." \
  -F "images[]=@pose_start.png" \
  -F "images[]=@pose_end.png" \
  -F "model=muse-video"
```

### 2.3 JSON Base64 Data URI or Gateway Image Reference
```bash
curl -X POST http://127.0.0.1:20129/muse-ai/v1/create-video \
  -H "Content-Type: application/json" \
  -d '{
    "prompt": "Animate this reference image with vibrant magical particle motion.",
    "model": "muse-video",
    "image": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
    "aspect_ratio": "16:9",
    "duration": 5
  }'
```

---

## 3. Verifying and Streaming the Video Media

Once the video is generated, you can inspect or stream the real MP4 media:

### Check Video Headers (`HEAD` or `curl -I`)

```bash
curl -I http://127.0.0.1:28130/muse-ai/v1/videos/video-6d4a9d57-5c61-408b-8825-a32047b83596/content
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
curl -o output.mp4 http://127.0.0.1:28130/muse-ai/v1/videos/video-6d4a9d57-5c61-408b-8825-a32047b83596/content
```

### Play in the Browser or Video Player

Open the player in your browser at:
`http://127.0.0.1:28130/video-template?job_id=video-6d4a9d57-5c61-408b-8825-a32047b83596`

---

## 4. Querying Video Job Details

You can query the durable job record anytime:

```bash
curl -s http://127.0.0.1:28130/muse-ai/v1/videos/video-6d4a9d57-5c61-408b-8825-a32047b83596
```

---

## 5. Asynchronous Submission (Optional)

If you prefer non-blocking submission and polling:

```bash
curl -X POST "http://127.0.0.1:28130/muse-ai/v1/create-video?async=true" \
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
curl -s http://127.0.0.1:28130/muse-ai/v1/videos/video-...
```

---

## 6. Handling Approvals (`409 Conflict`)

If the response returns `409 Conflict` with `MuseApprovalRequired`:
1. **Manual Resolution**: Open the session in your browser and grant permission. Then rerun the curl command with the same prompt.
2. **Headless Auto-Approval**: Enable headless Sentinel auto-approvals in the dashboard at `http://127.0.0.1:28130/dashboard` or via:
   ```bash
   curl -X POST http://127.0.0.1:28130/api/v1/dashboard/muse-config \
     -H "Content-Type: application/json" \
     -d '{"approval_mode": "on", "allowed_destinations": ["muse.ai"], "allowed_types": ["network"]}'
   ```
