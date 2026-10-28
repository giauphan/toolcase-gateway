# Echo Provider Module Example

A production-ready reference example of a self-contained provider module for **Toolcase Gateway**, built according to the platform module specification (see [`docs/MODULES.md`](../../../docs/MODULES.md)).

This module demonstrates how to implement an out-of-tree provider supporting OpenAI-compatible chat completions (`/chat/completions`) and model listings (`/models`) over the gateway's isolated `stdio-stream` protocol.

---

## 1. Directory Structure

```text
echo-provider/
├── module.json         # Declarative platform manifest
├── run.sh              # Executable entrypoint (marked chmod +x)
├── adapter.py          # Provider adapter logic
└── README.md           # Documentation & instructions
```

---

## 2. Manifest (`module.json`)

```json
{
  "id": "echo-provider",
  "name": "Echo Provider Adapter",
  "version": "1.0.0",
  "namespace": "/custom/echo",
  "entrypoint": "run.sh",
  "protocol": "stdio-stream",
  "capabilities": [
    "chat_completions",
    "models"
  ],
  "config_schema": {
    "prefix": {
      "type": "string",
      "default": "Echo"
    }
  }
}
```

- **`id`**: Unique identifier for the module (alphanumeric, `-`, `_`).
- **`namespace`**: Canonical URL prefix mounted by the gateway (e.g. `/custom/echo`). Requests matching `/custom/echo/*` are routed to this module.
- **`entrypoint`**: Executable file invoked by the gateway.
- **`protocol`**: `stdio-stream` for isolated child processes communicating via JSON lines.

---

## 3. Communication Protocol (`stdio-stream`)

When a request arrives at `/custom/echo/v1/chat/completions`, the gateway spawns `run.sh` in an isolated process sandbox and writes a single-line JSON payload to its standard input (`stdin`):

```json
{
  "method": "POST",
  "path": "/v1/chat/completions",
  "headers": {
    "content-type": "application/json"
  },
  "body": "{\"model\":\"echo-model\",\"messages\":[{\"role\":\"user\",\"content\":\"Ping\"}]}"
}
```

The adapter processes the request and writes a single-line JSON response to standard output (`stdout`):

```json
{
  "status": 200,
  "headers": {
    "content-type": "application/json"
  },
  "body": "{\"id\":\"chatcmpl-echo-12345678\",\"object\":\"chat.completion\",\"model\":\"echo-model\",\"choices\":[{\"index\":0,\"message\":{\"role\":\"assistant\",\"content\":\"Echo: Ping\"},\"finish_reason\":\"stop\"}]}"
}
```

---

## 4. Local Testing

You can test the adapter directly from your terminal without starting the gateway:

### Test Chat Completions
```sh
echo '{"method":"POST","path":"/v1/chat/completions","headers":{"content-type":"application/json"},"body":"{\"model\":\"echo-model\",\"messages\":[{\"role\":\"user\",\"content\":\"Ping\"}]}"}' | ./run.sh
```

### Test Models List
```sh
echo '{"method":"GET","path":"/v1/models","headers":{},"body":""}' | ./run.sh
```

---

## 5. How to Package into a ZIP Archive

Create a standard ZIP archive containing the module files:

```sh
cd examples/modules/echo-provider
zip -r echo-provider.zip module.json run.sh adapter.py
```

---

## 6. How to Install into Toolcase Gateway

### Option A: Via Web Dashboard (1-Click or Upload)

1. Open the dashboard at `http://localhost:20130/dashboard#modules-section`.
2. Scroll to the **Platform Gateway Modules** section.
3. Choose one of the following:
   - **System Catalog**: Click **Install** on any catalog item.
   - **ZIP Upload**: Under **Install Self-Contained Module**, select `echo-provider.zip` and click **Install Module**.
   - **GitHub Clone**: Enter the Git repository URL (e.g. `https://github.com/my-org/echo-provider`) and click **Install Module**.

### Option B: Via REST API (`curl`)

#### From Base64 ZIP:
```sh
ZIP_B64=$(base64 -w 0 echo-provider.zip)

curl -X POST http://127.0.0.1:20130/api/v1/dashboard/modules/install \
  -H "Content-Type: application/json" \
  -d '{
    "id": "echo-provider",
    "zip_base64": "'"$ZIP_B64"'"
  }'
```

#### From GitHub Repository:
```sh
curl -X POST http://127.0.0.1:20130/api/v1/dashboard/modules/install \
  -H "Content-Type: application/json" \
  -d '{
    "source": "https://github.com/my-org/echo-provider",
    "branch": "main"
  }'
```

### Option C: Direct Drop-in (Odoo / WordPress Style)

Copy the module folder directly into the gateway's state directory:

```sh
mkdir -p .gw_state/modules/echo-provider
cp -r examples/modules/echo-provider/* .gw_state/modules/echo-provider/
```

The gateway automatically discovers and mounts the module on boot or reload!

### Option D: Via MCP Tool

AI coding agents can install the module using the `gateway_install_personal_module` tool:

```json
{
  "name": "gateway_install_personal_module",
  "arguments": {
    "source": "https://github.com/my-org/echo-provider"
  }
}
```

---

## 7. Invoking the Installed Module

Once installed and active, send OpenAI-compatible requests directly to the module's mounted namespace:

```sh
curl -X POST http://127.0.0.1:20130/custom/echo/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "echo-model",
    "messages": [
      {
        "role": "user",
        "content": "Hello from custom module!"
      }
    ]
  }'
```

Response:
```json
{
  "id": "chatcmpl-echo-a1b2c3d4",
  "object": "chat.completion",
  "created": 1700000000,
  "model": "echo-model",
  "choices": [
    {
      "index": 0,
      "message": {
        "role": "assistant",
        "content": "Echo: Hello from custom module!"
      },
      "finish_reason": "stop"
    }
  ],
  "usage": {
    "prompt_tokens": 5,
    "completion_tokens": 6,
    "total_tokens": 11
  }
}
```
