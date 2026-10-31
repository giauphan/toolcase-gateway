# Video Scheduling & Facebook Auto-Translation Guide

This guide documents how to schedule video publications on **Facebook** and **TikTok**, and explains how the **Facebook Auto-Translation & AI Voice Dubbing** feature works across Toolcase Gateway.

---

## 1. Overview & Capabilities Matrix

| Feature | Facebook Suite (`/custom/facebook`) | TikTok Creator Studio (`/tiktok`) |
| :--- | :--- | :--- |
| **Direct Video Publish** | ✅ Supported (Graph API & Business Suite) | ✅ Supported (`/tiktok/v1/publish`) |
| **Scheduled Publishing** | ✅ Supported (`scheduled_publish_time`) | ✅ Supported (`schedule_time`) |
| **Scheduling Window** | 10 minutes to 75 days in advance | 15 minutes to 10 days in advance |
| **Auto-Translation / Dubbing** | ✅ **Enabled by default** (14 languages) | N/A (platform dependent) |
| **Custom Language Selection**| ✅ Supported (`translated_languages`) | N/A |
| **Status / Visibility Options**| Public, Scheduled, Unlisted | `only-me` (private), `public`, `friends` |

---

## 2. Facebook Video & Status Scheduling

Facebook supports scheduled publishing through both the **Meta Graph API** (`/videos` and `/feed`) and **Meta Business Suite GraphQL** (`BusinessComposerStoryCreationMutation`).

### 2.1 Scheduling Parameters
- **Parameter Name**: `scheduled_publish_time`
- **Format**: Unix timestamp (seconds since epoch in integer format).
- **Rules**:
  - Must be between **10 minutes** (`now + 600`) and **75 days** (`now + 6,480,000`) into the future.
  - When scheduling via Graph API, the video or post is marked with `"published": false`.
  - When scheduling via GraphQL composer, `variables.input.base.publish_time` is attached.

### 2.2 Schedule a Facebook Video via Graph Video API (`curl`)

```bash
# Calculate timestamp: e.g., 2 hours from now
SCHEDULE_TIME=$(($(date +%s) + 7200))

curl -X POST http://127.0.0.1:28130/custom/facebook/v1/upload/video \
  -H "Content-Type: application/json" \
  -d "{
    \"target_id\": \"1435449429646177\",
    \"title\": \"Scheduled Product Showcase\",
    \"description\": \"This video is scheduled for release in 2 hours.\",
    \"scheduled_publish_time\": ${SCHEDULE_TIME},
    \"video_base64\": \"$(base64 -w 0 path/to/video.mp4)\"
  }"
```

#### Response Example
```json
{
  "status": "success",
  "video_id": "1089594350750481",
  "target_id": "1435449429646177",
  "publish_result": {
    "id": "1089594350750481"
  }
}
```

### 2.3 Schedule a Status / Text Post on Facebook

```bash
curl -X POST http://127.0.0.1:28130/custom/facebook/v1/publish/status \
  -H "Content-Type: application/json" \
  -d "{
    \"target_id\": \"1435449429646177\",
    \"message\": \"Upcoming live stream scheduled for tonight!\",
    \"scheduled_publish_time\": ${SCHEDULE_TIME}
  }"
```

### 2.4 Verify Scheduled Post Status

Query the Facebook Graph object to confirm its scheduled status:

```bash
curl -s "https://graph.facebook.com/v21.0/1089594350750481?fields=id,title,published,scheduled_publish_time,publish_status&access_token=${PAGE_TOKEN}"
```

Expected output:
```json
{
  "id": "1089594350750481",
  "title": "Scheduled Product Showcase",
  "published": false,
  "scheduled_publish_time": 1793378504,
  "publish_status": "scheduled"
}
```

---

## 3. TikTok Video Scheduling

TikTok Creator Center supports scheduling posts for creator accounts with web scheduling capabilities enabled.

### 3.1 Scheduling Parameters
- **Parameter Name**: `schedule_time` (or `scheduled_publish_time`)
- **Format**: Unix timestamp (seconds since epoch in integer format).
- **Rules**:
  - Must be between **15 minutes** and **30 days** from the current moment (some regional accounts have a 10-day ceiling).
  - Video visibility is automatically set to `"public"` (`visibility_type: 0`) because TikTok does not permit private scheduled posts.
  - Supports 1-step direct publishing by providing `video_path` (e.g., `"/path/to/video.mp4"`), eliminating manual TOS chunking.
  - Toolcase Gateway automatically performs the upstream `/api/v1/post_schedule/ack/?ack_type=2` server storage acknowledgement, embedding `schedule_time` into `feature_common_info_list` alongside Creator Center parameters.

### 3.2 Schedule a TikTok Post (`curl`)

```bash
# Calculate timestamp: e.g., 1 day from now
TIKTOK_SCHEDULE=$(($(date +%s) + 86400))

# 1-Step Schedule using local video file path
curl -X POST http://127.0.0.1:28130/tiktok/v1/publish \
  -H "Content-Type: application/json" \
  -d "{
    \"video_path\": \"/path/to/video.mp4\",
    \"text\": \"Tomorrow morning routine highlights #daily #routine\",
    \"schedule_time\": ${TIKTOK_SCHEDULE}
  }"
```

#### Response Example
```json
{
  "project_id": "7702784686095111175",
  "item_id": "7702784742617337095",
  "project_status": 1,
  "status": "scheduled",
  "visibility_type": 0,
  "visibility": "public",
  "schedule_time": 1793613600,
  "status_code": 0,
  "status_msg": ""
}
```

> [!NOTE]
> Toolcase Gateway automatically authorizes server-side video staging via TikTok's schedule acknowledgment API (`/api/v1/post_schedule/ack/`). If your account is restricted by TikTok trust and safety from scheduling, verify in TikTok Creator Center settings that creator upload features are active.

---

## 4. Facebook Video Auto-Translation & AI Voice Dubbing

Meta Business Suite includes automated AI voice dubbing and multilingual translation for uploaded video content. Toolcase Gateway has this capability **turned on by default** for video publications.

### 4.1 How It Works
When a video is submitted to Meta Business Suite's composer (`BusinessComposerStoryCreationMutation`), the adapter attaches `translatedAudioMetadata` to the video attachment specifications:

```json
{
  "input": {
    "base": {
      "attachments": [
        {
          "video": {
            "id": "<video_id>",
            "additional_video_metadata": {
              "translatedAudioMetadata": [
                {
                  "audio_handle": "",
                  "enabled": true,
                  "is_lipsync_enabled": false,
                  "lang_code": "en_US",
                  "require_review": false,
                  "source_lang_code": ""
                },
                ...
              ]
            }
          }
        }
      ]
    }
  }
}
```

### 4.2 Default Supported Languages (14 Languages)
By default, the adapter automatically enables auto-dubbing tracks for the following 14 languages:

1. `en_US` — English (US)
2. `vi_VN` — Vietnamese
3. `hi_IN` — Hindi
4. `ar_AR` — Arabic
5. `pt_BR` — Portuguese (Brazil)
6. `es_LA` — Spanish (Latin America)
7. `fr_FR` — French
8. `bn_IN` — Bengali
9. `id_ID` — Indonesian
10. `th_TH` — Thai
11. `te_IN` — Telugu
12. `ta_IN` — Tamil
13. `mr_IN` — Marathi
14. `kn_IN` — Kannada

### 4.3 Publishing with Default Auto-Translation
No special parameters are needed! Any call to publish a video automatically includes all 14 languages enabled:

```bash
curl -X POST http://127.0.0.1:28130/custom/facebook/v1/publish \
  -H "Content-Type: application/json" \
  -d '{
    "target_id": "1435449429646177",
    "video_id": "2035111870485874",
    "title": "International AI Tech Talk",
    "description": "Explaining Clean Architecture with automated multi-language dubbing."
  }'
```

### 4.4 Customizing Target Languages
To select specific languages instead of the full default set, pass the `translated_languages` array:

```bash
curl -X POST http://127.0.0.1:28130/custom/facebook/v1/publish \
  -H "Content-Type: application/json" \
  -d '{
    "target_id": "1435449429646177",
    "video_id": "2035111870485874",
    "title": "Targeted Regional Broadcast",
    "enable_auto_translate": true,
    "translated_languages": ["en_US", "vi_VN", "es_LA"]
  }'
```

### 4.5 Disabling Auto-Translation
If you do not want automated translations for a specific video or globally:

#### Per-Request Override:
```json
{
  "target_id": "1435449429646177",
  "video_id": "2035111870485874",
  "enable_auto_translate": false
}
```

#### Global Addon Configuration:
Set `"auto_translate": false` in `addons/facebook-suite/config.json` or update via API:

```bash
curl -X POST http://127.0.0.1:28130/custom/facebook/v1/config \
  -H "Content-Type: application/json" \
  -d '{"auto_translate": false}'
```

---

## 5. Unified Python Scheduling & Dubbing Example

Below is a complete, runnable script illustrating scheduled posting on Facebook and TikTok, with Facebook auto-translate active:

```python
import json
import time
import urllib.request

GATEWAY_BASE = "http://127.0.0.1:28130"

def post_json(endpoint: str, data: dict) -> dict:
    url = f"{GATEWAY_BASE}{endpoint}"
    req = urllib.request.Request(
        url,
        data=json.dumps(data).encode("utf-8"),
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req) as resp:
        return json.loads(resp.read().decode("utf-8"))

# Schedule 3 hours ahead
scheduled_time = int(time.time()) + 10800

# 1. Schedule on Facebook with Auto-Dubbing enabled
fb_response = post_json(
    "/custom/facebook/v1/publish",
    {
        "target_id": "1435449429646177",
        "video_id": "2035111870485874",
        "title": "Global Product Announcement",
        "description": "Multi-language AI video with automated audio translation.",
        "scheduled_publish_time": scheduled_time,
        "enable_auto_translate": True,
    },
)
print("Facebook Scheduled Result:", fb_response)

# 2. Schedule on TikTok (using local file path or video_id)
tiktok_response = post_json(
    "/tiktok/v1/publish",
    {
        "video_path": "render.mp4",
        "text": "Scheduled premier for our global audience! #innovation #future",
        "schedule_time": scheduled_time,
    },
)
print("TikTok Scheduled Result:", tiktok_response)
```

---

## 6. Troubleshooting & FAQs

### Q: Why does scheduled status show `"published": false` on Facebook?
Facebook keeps scheduled videos with `"published": false` and `"publish_status": "scheduled"` until the exact timestamp arrives, at which point Facebook's backend automatically changes the status to `"published": true` and publishes the video to the Page feed.

### Q: How can I verify that auto-translation was generated?
Navigate to **Meta Business Suite** -> **Content** -> **Published / Scheduled Videos**. Click on the video and open **Audio & Captions**. You will see the generated localized audio tracks for each of the 14 languages.

### Q: Can I schedule a post for less than 10 minutes?
No. Meta's minimum scheduling window is 10 minutes in advance. If a timestamp is under 10 minutes, Meta rejects the request with an invalid parameter error.
