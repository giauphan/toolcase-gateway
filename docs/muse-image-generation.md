# Muse Image Generation

## 1. Overview

`toolcase-gateway` provides end-to-end text-to-image generation powered by Muse AI Confidential VMs (CVM) at `/muse-ai/v1/create-image` (also aliased at `/muse-ai/create-image` and OpenAI-compatible `/v1/images/generations`).

### Key Features
- **Configurable Aspect Ratios**: Choose from `1:1` (default), `16:9`, `9:16`, `4:3`, `3:4`, `3:2`, `2:3`, `5:4`, `4:5`.
- **OpenAI Size Compatibility**: Accepts standard OpenAI `size` fields (e.g. `1024x1024` &rarr; `1:1`, `1792x1024` &rarr; `16:9`, `1024x1792` &rarr; `9:16`, `1024x768` &rarr; `4:3`).
- **Encrypted Noise XX Media Fetching**: When Muse CVM generates images in its internal sandbox workspace (`sandbox://workspace/...`), the gateway transparently fetches the binary media over the encrypted Noise XX session and caches it under `$GW_STATE_DIR/media/{id}.{ext}`.
- **Direct Preview & Download Links**:
  - `preview_url` (`/muse-ai/v1/images/{id}/content`): Serves the image inline (`Content-Disposition: inline`) for browser rendering.
  - `download_url` (`/muse-ai/v1/images/{id}/download`): Serves the image as a downloadable attachment (`Content-Disposition: attachment`).
- **Interactive Web Studio (`/image-template`)**: A built-in web studio allowing users to interactively choose aspect ratios, submit prompts, preview artwork directly in the browser, and download generated image files.
- **Synchronous & Asynchronous Execution**:
  - Synchronous mode (`?wait=true`, `curl` default) waits for upstream image creation and returns the validated image URL directly in `image_url`, `preview_url`, `download_url`, and OpenAI-compatible `data[0].url`.
  - Asynchronous mode (`?async=true`) returns `status: "submitting"` immediately with an allocated `image-<uuid>` job ID.
- **Job Status Tracking**: Inspect generation status anytime at `GET /muse-ai/v1/images/{id}`.
- **Fail-Closed Validation**: Rejects invalid prompts or unsupported aspect ratios with clear client error messages.

---

## 2. API Endpoints

### 2.1 Interactive Web Studio (`GET /image-template`)

Open in any browser:
```
http://127.0.0.1:20129/image-template
```
Features:
- Select aspect ratios via clickable pills: `1:1`, `16:9`, `9:16`, `4:3`, `3:2`.
- Enter prompt descriptions.
- Instant inline image preview viewer with responsive aspect ratio frames.
- Direct "Download Image" button.

---

### 2.2 Create Image (`POST /muse-ai/v1/create-image` or `POST /v1/images/generations`)

#### Synchronous Request via Muse API
```bash
curl -X POST "http://127.0.0.1:20129/muse-ai/v1/create-image?wait=true" \
  -H "Content-Type: application/json" \
  -d '{
    "prompt": "A majestic golden eagle soaring over snow-capped alpine mountains at sunrise",
    "aspect_ratio": "16:9"
  }'
```

#### Synchronous Request via OpenAI API Alias
```bash
curl -X POST "http://127.0.0.1:20129/v1/images/generations" \
  -H "Content-Type: application/json" \
  -d '{
    "prompt": "A majestic golden eagle soaring over snow-capped alpine mountains at sunrise",
    "size": "1792x1024"
  }'
```

#### Synchronous Response (`200 OK`)
```json
{
  "id": "image-c63c6257-5803-4452-8991-a0f459736b46",
  "object": "image.generation",
  "created": 1793029718,
  "provider": "muse_chat",
  "resolved_provider": "muse_chat",
  "requested_model": "muse-image",
  "resolved_model": "muse-image",
  "model": "muse-image",
  "prompt": "A majestic golden eagle soaring over snow-capped alpine mountains at sunrise",
  "aspect_ratio": "16:9",
  "requested_aspect_ratio": "16:9",
  "status": "completed",
  "output_verified": true,
  "media_validated": true,
  "image_url": "/muse-ai/v1/images/image-c63c6257-5803-4452-8991-a0f459736b46/content",
  "preview_url": "/muse-ai/v1/images/image-c63c6257-5803-4452-8991-a0f459736b46/content",
  "download_url": "/muse-ai/v1/images/image-c63c6257-5803-4452-8991-a0f459736b46/download",
  "full_image_url": "http://127.0.0.1:20129/muse-ai/v1/images/image-c63c6257-5803-4452-8991-a0f459736b46/content",
  "full_preview_url": "http://127.0.0.1:20129/muse-ai/v1/images/image-c63c6257-5803-4452-8991-a0f459736b46/content",
  "full_download_url": "http://127.0.0.1:20129/muse-ai/v1/images/image-c63c6257-5803-4452-8991-a0f459736b46/download",
  "data": [
    {
      "url": "/muse-ai/v1/images/image-c63c6257-5803-4452-8991-a0f459736b46/content"
    }
  ]
}
```

---

### 2.3 Preview Image Inline (`GET /muse-ai/v1/images/{id}/content`)

Direct link for browser preview:
- Sends `Content-Disposition: inline; filename="{id}.webp"`.
- Streams the local cached image bytes directly with accurate MIME type (`image/webp`, `image/png`, `image/jpeg`).
- If external public URL, automatically redirects with `HTTP 302 Found`.

```bash
curl -I "http://127.0.0.1:20129/muse-ai/v1/images/image-c63c6257-5803-4452-8991-a0f459736b46/content"
```

Response:
```http
HTTP/1.1 200 OK
Content-Type: image/webp
Content-Length: 561478
Accept-Ranges: bytes
Access-Control-Allow-Origin: *
Access-Control-Allow-Headers: Range, Content-Type, Accept
Content-Disposition: inline; filename="image-c63c6257-5803-4452-8991-a0f459736b46.webp"
Connection: close
```

---

### 2.4 Download Image File (`GET /muse-ai/v1/images/{id}/download`)

Direct link for file download:
- Sends `Content-Disposition: attachment; filename="{id}.webp"`.
- Prompts browser file save dialog directly.

```bash
curl -O -J "http://127.0.0.1:20129/muse-ai/v1/images/image-c63c6257-5803-4452-8991-a0f459736b46/download"
```

---

### 2.5 Get Image Job Status (`GET /muse-ai/v1/images/{id}`)

Query the status of an ongoing or completed image generation task:

```bash
curl -X GET "http://127.0.0.1:20129/muse-ai/v1/images/image-c63c6257-5803-4452-8991-a0f459736b46"
```

---

## 3. Supported Aspect Ratios & Sizes

| Aspect Ratio | Standard Size Equivalent | Orientation |
|---|---|---|
| `1:1` *(default)* | `1024x1024`, `512x512` | Square |
| `16:9` | `1792x1024`, `1920x1080` | Landscape / Widescreen |
| `9:16` | `1024x1792`, `1080x1920` | Portrait / Mobile |
| `4:3` | `1024x768` | Landscape |
| `3:4` | `768x1024` | Portrait |
| `3:2` | `1200x800` | Classic 35mm photo |
| `2:3` | `800x1200` | Portrait 35mm photo |
| `5:4` | `1280x1024` | Traditional print |
| `4:5` | `1024x1280` | Social portrait |

```bash
curl -O -J "http://127.0.0.1:28130/muse-ai/v1/images/image-9457fc85-5b87-43cf-be64-fbbef273eefb/download"
```

---

### 2.5 Get Image Job Status (`GET /muse-ai/v1/images/{id}`)

Query the status of an ongoing or completed image generation task:

```bash
curl -X GET "http://127.0.0.1:28130/muse-ai/v1/images/image-9457fc85-5b87-43cf-be64-fbbef273eefb"
```

---

## 3. Supported Aspect Ratios & Sizes

| Aspect Ratio | Standard Size Equivalent | Orientation |
|---|---|---|
| `1:1` *(default)* | `1024x1024`, `512x512` | Square |
| `16:9` | `1792x1024`, `1920x1080` | Landscape / Widescreen |
| `9:16` | `1024x1792`, `1080x1920` | Portrait / Mobile |
| `4:3` | `1024x768` | Landscape |
| `3:4` | `768x1024` | Portrait |
| `3:2` | `1200x800` | Classic 35mm photo |
| `2:3` | `800x1200` | Portrait 35mm photo |
| `5:4` | `1280x1024` | Traditional print |
| `4:5` | `1024x1280` | Social portrait |
