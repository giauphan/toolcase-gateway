# Muse Video Curl Test

Use this smoke test when the gateway is already running and `.env` contains a valid `GW_MUSEAI_COOKIE`.

Keep the JSON payload on one physical line. Real newlines inside the `prompt` string make the body invalid JSON.

```bash
curl --no-buffer --max-time 930 -H 'Content-Type: application/json' --data-raw '{"prompt":"Create a cheerful cartoon-style short video: a small orange fox wearing a blue scarf rides a skateboard through a colorful city park, bouncy animation, bright morning light, playful camera moves, whimsical clouds, no text, no logos, family-friendly.","model":"muse-video","aspect_ratio":"16:9","duration":5}' http://127.0.0.1:20130/muse-ai/v1/create-video
```

Successful response includes `status: "completed"` and `video_url`.

If response is `409 Conflict` with scoped approval pending, open Muse in browser, grant permission, then retry the same curl command.
