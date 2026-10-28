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

JSON_CONTENT_TYPE = "application/json"

def parse_body(raw_body):
    if not raw_body:
        return {}
    try:
        return json.loads(raw_body)
    except Exception:
        return {}

def extract_last_user_message(messages):
    for msg in reversed(messages):
        if msg.get("role") == "user":
            return msg.get("content", "")
    if messages:
        return messages[-1].get("content", "")
    return ""

def handle_models():
    models_resp = {
        "object": "list",
        "data": [
            {
                "id": "echo-model",
                "object": "model",
                "created": int(time.time()),
                "owned_by": "custom-echo",
            }
        ],
    }
    return {
        "status": 200,
        "headers": {"content-type": JSON_CONTENT_TYPE},
        "body": json.dumps(models_resp),
    }

def handle_chat_completions(body_data):
    messages = body_data.get("messages", [])
    last_message = extract_last_user_message(messages)
    completion_content = (
        f"Echo: {last_message}"
        if last_message
        else "Echo: Hello from example module!"
    )

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
                    "content": completion_content,
                },
                "finish_reason": "stop",
            }
        ],
        "usage": {
            "prompt_tokens": len(last_message.split()),
            "completion_tokens": len(completion_content.split()),
            "total_tokens": len(last_message.split()) + len(completion_content.split()),
        },
    }
    return {
        "status": 200,
        "headers": {"content-type": JSON_CONTENT_TYPE},
        "body": json.dumps(chat_resp),
    }

def handle_fallback(method, path):
    fallback = {
        "module": "echo-provider",
        "status": "ok",
        "method": method,
        "path": path,
    }
    return {
        "status": 200,
        "headers": {"content-type": JSON_CONTENT_TYPE},
        "body": json.dumps(fallback),
    }

def handle_request(req):
    method = req.get("method", "GET").upper()
    path = req.get("path", "")
    body_data = parse_body(req.get("body", ""))

    if path.endswith("/models"):
        return handle_models()
    if path.endswith("/chat/completions") or method == "POST":
        return handle_chat_completions(body_data)
    return handle_fallback(method, path)

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
                "headers": {"content-type": JSON_CONTENT_TYPE},
                "body": json.dumps({"error": str(e)}),
            }
        sys.stdout.write(json.dumps(res) + "\n")
        sys.stdout.flush()

if __name__ == "__main__":
    main()
