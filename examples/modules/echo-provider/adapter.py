#!/usr/bin/env python3
"""
Example Echo Provider Adapter for Toolcase Gateway.

This module demonstrates how an out-of-tree provider communicates with
Toolcase Gateway using the stdio-stream protocol.
"""

import sys
import json
import time
import uuid

def handle_request(req):
    method = req.get("method", "GET").upper()
    path = req.get("path", "")
    raw_body = req.get("body", "")

    # Parse body if present
    body_data = {}
    if raw_body:
        try:
            body_data = json.loads(raw_body)
        except Exception:
            body_data = {}

    # 1. Models endpoint
    if path.endswith("/models"):
        models_resp = {
            "object": "list",
            "data": [
                {
                    "id": "echo-model",
                    "object": "model",
                    "created": int(time.time()),
                    "owned_by": "custom-echo"
                }
            ]
        }
        return {
            "status": 200,
            "headers": {"content-type": "application/json"},
            "body": json.dumps(models_resp)
        }

    # 2. Chat completions endpoint
    if path.endswith("/chat/completions") or method == "POST":
        messages = body_data.get("messages", [])
        last_message = ""
        for msg in reversed(messages):
            if msg.get("role") == "user":
                last_message = msg.get("content", "")
                break

        if not last_message and messages:
            last_message = messages[-1].get("content", "")

        completion_content = f"Echo: {last_message}" if last_message else "Echo: Hello from example module!"

        chat_resp = {
            "id": f"chatcmpl-echo-{uuid.uuid4().hex[:8]}",
            "object": "chat.completion",
            "created": int(time.time()),
            "model": body_data.get("model", "echo-model"),
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": completion_content
                    },
                    "finish_reason": "stop"
                }
            ],
            "usage": {
                "prompt_tokens": len(last_message.split()),
                "completion_tokens": len(completion_content.split()),
                "total_tokens": len(last_message.split()) + len(completion_content.split())
            }
        }
        return {
            "status": 200,
            "headers": {"content-type": "application/json"},
            "body": json.dumps(chat_resp)
        }

    # Default fallback
    fallback = {
        "module": "echo-provider",
        "status": "ok",
        "method": method,
        "path": path
    }
    return {
        "status": 200,
        "headers": {"content-type": "application/json"},
        "body": json.dumps(fallback)
    }

def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
            res = handle_request(req)
        except Exception as e:
            res = {
                "status": 500,
                "headers": {"content-type": "application/json"},
                "body": json.dumps({"error": str(e)})
            }
        sys.stdout.write(json.dumps(res) + "\n")
        sys.stdout.flush()

if __name__ == "__main__":
    main()
