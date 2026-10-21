#!/usr/bin/env python3
"""
Benchmark comparison harness for toolcase-gateway routing modes:
  A: Baseline (existing)
  B: Rules (capability-aware deterministic)
  C: Jev (decision engine / shadow mode)
"""

import argparse
import ipaddress
import json
import time
import urllib.request
from urllib.parse import urlparse
from typing import Dict, Any, List

SAMPLE_TASKS = [
    {
        "id": "task_simple_fix",
        "description": "Fix typo in docstring",
        "has_tools": False,
        "payload": {
            "model": "claude-3-5-haiku",
            "messages": [{"role": "user", "content": "Fix typo in docstring"}],
        },
    },
    {
        "id": "task_refactor_arch",
        "description": "Refactor multi-file HTTP reverse proxy",
        "has_tools": True,
        "payload": {
            "model": "claude-3-5-sonnet",
            "messages": [{"role": "user", "content": "Refactor router modules"}],
            "tools": [{"type": "function", "function": {"name": "read_file"}}],
        },
    },
]

def _validate_endpoint(url: str) -> str:
    parsed = urlparse(url)
    if parsed.scheme not in ("http", "https"):
        raise ValueError(f"Invalid URL scheme: '{parsed.scheme}'. Only http and https are allowed.")
    if not parsed.hostname:
        raise ValueError("Invalid URL: hostname is required.")

    if parsed.hostname in ("169.254.169.254", "metadata.google.internal"):
        raise ValueError(f"Access to metadata endpoint '{parsed.hostname}' is forbidden.")

    try:
        ip = ipaddress.ip_address(parsed.hostname)
        if ip.is_link_local:
            raise ValueError(f"Access to link-local address '{parsed.hostname}' is forbidden.")
    except ValueError:
        pass

    return url.rstrip("/")


def benchmark_endpoint(name: str, base_url: str, tasks: List[Dict[str, Any]]) -> Dict[str, Any]:
    safe_base = _validate_endpoint(base_url)
    latencies = []
    successes = 0
    total = len(tasks)

    for task in tasks:
        start = time.perf_counter()
        req_data = json.dumps(task["payload"]).encode("utf-8")
        req = urllib.request.Request(
            f"{safe_base}/v1/chat/completions",
            data=req_data,
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        try:
            with urllib.request.urlopen(req, timeout=5) as response:
                if 200 <= response.status < 300:
                    successes += 1
        except Exception:
            pass
        latencies.append((time.perf_counter() - start) * 1000.0)

    avg_ms = sum(latencies) / len(latencies) if latencies else 0.0
    return {
        "mode": name,
        "total_requests": total,
        "successful_requests": successes,
        "average_latency_ms": round(avg_ms, 2),
        "min_latency_ms": round(min(latencies), 2) if latencies else 0.0,
        "max_latency_ms": round(max(latencies), 2) if latencies else 0.0,
    }

def main():
    parser = argparse.ArgumentParser(description="Toolcase Gateway Routing Benchmark")
    parser.add_argument("--endpoint", default="http://127.0.0.1:20130", type=_validate_endpoint, help="Gateway URL")
    parser.add_argument("--mode", default="A", choices=["A", "B", "C"], help="Mode identifier")
    args = parser.parse_args()

    result = benchmark_endpoint(f"Mode_{args.mode}", args.endpoint, SAMPLE_TASKS)
    print(json.dumps(result, indent=2))

if __name__ == "__main__":
    main()
