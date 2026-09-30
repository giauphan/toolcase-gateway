# Models Reference

Toolcase Gateway exposes an OpenAI-compatible API translating standard chat completions requests to diverse upstream protocols, such as Prism and Muse.ai Confidential VMs (CVM).

---

## Model Catalog

| Model ID | Provider / Engine | Protocol | Reasoning / Effort | Catalog Route |
| :--- | :--- | :--- | :--- | :--- |
| `gpt-5.6-sol` | OpenAI / Prism | HTTP / OmniRoute | Default | `/v1/models` |
| `gpt-5.6-sol-low` | OpenAI / Prism | HTTP / OmniRoute | Low Effort | `/v1/models` |
| `gpt-5.6-sol-medium` | OpenAI / Prism | HTTP / OmniRoute | Medium Effort | `/v1/models` |
| `gpt-5.6-sol-high` | OpenAI / Prism | HTTP / OmniRoute | High Effort | `/v1/models` |
| `gpt-5.6-sol-xhigh` | OpenAI / Prism | HTTP / OmniRoute | Extra High Effort | `/v1/models` |
| `muse` | Muse.ai | Noise XX / WebSocket | Default | `/v1/models`, `/muse-ai/v1/models` |

---

## 1. OpenAI / Prism Models

These models map requests to the upstream Prism OpenAI platform using standard JSON payloads, failover candidate sequences, and tool-name repairs.

### Model IDs
- `gpt-5.6-sol`: Default model for general text and tool use.
- `gpt-5.6-sol-{low|medium|high|xhigh}`: Reasoning effort variants for deep problem solving.

### Endpoints
- `POST /v1/chat/completions` (OpenAI format, supports automatic failovers)
- `POST /prism-openai/v1/chat/completions` (Dedicated raw endpoint for Prism)

### Example Request
```bash
curl -X POST http://127.0.0.1:20130/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-5.6-sol-high",
    "messages": [
      {"role": "user", "content": "Solve this equation step-by-step: 2x + 5 = 15"}
    ]
  }'
```

---

## 2. Muse.ai Model (`muse`)

The `muse` model routes standard OpenAI requests through a native, end-to-end encrypted WebSocket connection powered by the **Noise Protocol Framework (`Noise_XX_25519_AESGCM_SHA256`)**.

### How It Works Under the Hood
1. **VM Wake & Hatch Token:** Automatically wakes the remote Confidential VM via `POST /api/hatch/vm/wake` and acquires notary tokens from `POST /api/hatch/token`.
2. **Noise XX Handshake:** Executes a 3-way cryptographic handshake with mutual attestation (`Message 3` includes a 32-byte randomized RV key and notary credentials).
3. **Protobuf Framing:** Encapsulates JSON chat payloads into binary `ApplicationRequest`, `ServiceFrame`, `ServiceRequest`, and `NoiseTransportFrame` buffers.
4. **Multiplexed Streams:** 
   - Establishes a background subscription via `POST /chat/subscribe` on Stream 2.
   - Dispatches user instructions via `POST /chat/stream` on Stream 1.
5. **Streaming Assembly:** Gathers `delta.text_append` event payloads and unrolls array/multi-turn message histories cleanly into standard OpenAI completion responses.

### Endpoints
- `POST /v1/chat/completions` (Specify `"model": "muse"`)
- `GET /muse-ai/v1/models` (Scoped catalog returning only `muse`)

### Example Request
```bash
curl -X POST http://127.0.0.1:20130/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "muse",
    "messages": [
      {"role": "user", "content": "Reply with MUSE_OK exactly"}
    ]
  }'
```

---

## Model Routing Matrix & Scope

- `GET /v1/models` lists all available global models (`gpt-5.6-sol*` variants and `muse`).
- `GET /muse-ai/v1/models` scopes specifically to the `muse` engine model.
- If upstream `muse` connection fails, OmniRoute automatically falls back to configured fallback candidates in `GW_FALLBACK_MODELS`.

---

## 3. Muse.ai Video Generation (`create-video`)

The gateway provides a distinct wrapper endpoint for upstream video generation using Muse.ai backend capabilities.

### Endpoint
`POST /muse-ai/v1/create-video`

### Request Parameters (JSON)
- `prompt` (string, **required**): The descriptive text describing the video to generate. Must not be empty.
- `model` (string, optional): Target video model. Supported values: `gen-3` (default), `gen-2`, `kling`.
- `aspect_ratio` (string, optional): Desired aspect ratio. Supported values: `16:9` (default), `9:16`, `1:1`, `5:4`, `4:3`.
- `duration` (number or string, optional): Generation duration in seconds (e.g., `5`, `10`, `30`, `60`, `90`, `120`). Defaults to `5`.

### Response Schema
Success returns `200 OK` with the following structure:
```json
{
  "id": "video-a1b2c3d4...",
  "object": "video.generation",
  "created": 1700000000,
  "model": "gen-3",
  "prompt": "flying over glowing neon mountains at night",
  "aspect_ratio": "16:9",
  "duration": 5,
  "status": "completed",
  "video_url": "https://cdn.muse.ai/video/xyz123.mp4"
}
```
*Note*: If the video generation requires more time and no immediate artifact URL is extracted during the response stream, `status` will be `pending` instead of `completed`, and `video_url` will be empty.

### Error Handling
- `400 Bad Request`: Missing prompt, empty string provided, or unsupported parameter given (e.g., passing `4:5` as `aspect_ratio` or `unsupported-model`).
- `401 Unauthorized`: Muse.ai session (`cookie`, `access_token`, etc.) is missing or expired.
- `502 Bad Gateway`: Upstream protocol connection over Noise WS failed or thread orchestration failed.

### Architecture: Thread Isolation & Tracking
For robust upstream generation, `create-video` leverages explicit thread isolation logic:
1. **Thread Spin-Up**: The handler triggers a `POST /thread/new` HTTP request with `x-nextjs-data: true` to the upstream server and captures the generated thread ID.
2. **Channel Subscription**: It uses the Noise WebSocket to subscribe to the isolated `thread:<thread_id>` instead of the default `main` channel.
3. **Payload Construction**: The prompt, model, and aspect ratio are concatenated into standard LLM messaging sequences and dispatched securely to trigger generation without alerting or polluting the primary connection context.
4. **URL Extraction**: Streaming text output from the assistant model is inspected continuously to securely resolve media URLs (`.mp4`, `.mov`, `.webm`).
