# Muse Image Generation

## 1. Overview

`toolcase-gateway` provides image generation capabilities powered by Muse.ai at `/muse-ai/v1/create-image` (also aliased at `/muse-ai/create-image` and OpenAI-compatible `/v1/images/generations`).

### Key Features
- **Configurable Aspect Ratios**: Choose from `1:1`, `16:9`, `9:16`, `4:3`, `3:4`, `3:2`, `2:3`, `5:4`, `4:5` (defaults to `1:1`).
- **OpenAI Size Compatibility**: Accepts standard OpenAI `size` fields (e.g. `1024x1024` -> `1:1`, `1792x1024` -> `16:9`, `1024x1792` -> `9:16`).
- **Direct Preview & Download Links**:
  - `preview_url` (`/muse-ai/v1/images/{id}/content`): Serves the image inline (`Content-Disposition: inline`) for browser display.
  - `download_url` (`/muse-ai/v1/images/{id}/download`): Serves the image as a downloadable attachment (`Content-Disposition: attachment`).
- **Interactive Web Studio (`/image-template`)**: A built-in web studio allowing users to interactively choose aspect ratios, submit prompts, preview the artwork directly in the browser, and download the image file.
- **Synchronous & Asynchronous Execution**:
  - Synchronous mode (`?wait=true`, `curl` default) waits for upstream image creation and returns the validated image URL directly in `image_url` and OpenAI-compatible `data[0].url`.
  - Asynchronous mode (`?async=true`) returns `status: "submitting"` immediately with an allocated `image-<uuid>` job ID.
- **Job Status Tracking**: Inspect generation status anytime at `GET /muse-ai/v1/images/{id}`.
- **Fail-Closed Validation**: Rejects invalid prompts or unsupported aspect ratios with clear client errors.

---

## 2. API Endpoints

### 2.1 Interactive Web Studio (`GET /image-template`)

Open in any browser:
```
http://127.0.0.1:20130/image-template
```
Features:
- Select aspect ratios via clickable pills: `1:1`, `16:9`, `9:16`, `4:3`, `3:2`.
- Enter prompt descriptions.
- Instant inline image preview viewer.
- Direct "Download Image" button.

---

### 2.2 Create Image (`POST /muse-ai/v1/create-image` or `POST /v1/images/generations`)

#### Synchronous Request Example
```bash
curl -X POST "http://127.0.0.1:20130/muse-ai/v1/create-image?wait=true" \
  -H "Content-Type: application/json" \
  -d '{
    "prompt": "A magical glowing tree in an enchanted forest at twilight, volumetric light, highly detailed digital painting",
    "aspect_ratio": "16:9"
  }'
```

#### Synchronous Response (`200 OK`)
```json
{
  "id": "image-9457fc85-5b87-43cf-be64-fbbef273eefb",
  "object": "image.generation",
  "created": 1729944000,
  "provider": "muse_chat",
  "resolved_provider": "muse_chat",
  "requested_model": "muse-image",
  "resolved_model": "muse-image",
  "model": "muse-image",
  "prompt": "A magical glowing tree in an enchanted forest at twilight...",
  "aspect_ratio": "16:9",
  "requested_aspect_ratio": "16:9",
  "status": "completed",
  "output_verified": true,
  "media_validated": true,
  "image_url": "https://muse.ai/files/123/456/magical_tree.png",
  "preview_url": "/muse-ai/v1/images/image-9457fc85-5b87-43cf-be64-fbbef273eefb/content",
  "download_url": "/muse-ai/v1/images/image-9457fc85-5b87-43cf-be64-fbbef273eefb/download",
  "full_image_url": "https://muse.ai/files/123/456/magical_tree.png",
  "full_preview_url": "http://127.0.0.1:20130/muse-ai/v1/images/image-9457fc85-5b87-43cf-be64-fbbef273eefb/content",
  "full_download_url": "http://127.0.0.1:20130/muse-ai/v1/images/image-9457fc85-5b87-43cf-be64-fbbef273eefb/download",
  "data": [
    {
      "url": "https://muse.ai/files/123/456/magical_tree.png"
    }
  ]
}
```

---

### 2.3 Preview Image Inline (`GET /muse-ai/v1/images/{id}/content`)

Direct link for browser preview:
- Sends `Content-Disposition: inline; filename="{id}.png"`.
- If stored locally, streams the image bytes directly.
- If external URL, automatically redirects with `HTTP 302 Found`.

```bash
curl -I "http://127.0.0.1:20130/muse-ai/v1/images/image-9457fc85-5b87-43cf-be64-fbbef273eefb/content"
```

---

### 2.4 Download Image File (`GET /muse-ai/v1/images/{id}/download`)

Direct link for file download:
- Sends `Content-Disposition: attachment; filename="{id}.png"`.

```bash
curl -O -J "http://127.0.0.1:20130/muse-ai/v1/images/image-9457fc85-5b87-43cf-be64-fbbef273eefb/download"
```

---

### 2.5 Get Image Job Status (`GET /muse-ai/v1/images/{id}`)

Query the status of an ongoing or completed image generation task:

```bash
curl -X GET "http://127.0.0.1:20130/muse-ai/v1/images/image-9457fc85-5b87-43cf-be64-fbbef273eefb"
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
