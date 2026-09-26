#!/usr/bin/env python3
"""Extract local Muse.ai session config from a user-owned HAR or cookie file."""

from __future__ import annotations

import argparse
import base64
import json
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit


_CONFIG_KEYS = (
    "access_token",
    "ws_token",
    "websocket_token",
    "notary_token",
    "token",
)
_WS_URL_KEYS = ("endpoint_url", "websocket_url", "ws_url", "websocket_endpoint")


def _walk(value: Any):
    if isinstance(value, dict):
        for key, child in value.items():
            yield key, child
            yield from _walk(child)
    elif isinstance(value, list):
        for child in value:
            yield from _walk(child)


def _response_json(entry: dict[str, Any]) -> Any | None:
    content = entry.get("response", {}).get("content", {})
    text = content.get("text")
    if not isinstance(text, str):
        return None
    if content.get("encoding") == "base64":
        try:
            text = base64.b64decode(text).decode("utf-8")
        except (ValueError, UnicodeDecodeError):
            return None
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        return None


def _header(headers: list[dict[str, Any]], name: str) -> str | None:
    for header in headers:
        if str(header.get("name", "")).lower() == name.lower():
            value = header.get("value")
            if isinstance(value, str) and value.strip():
                return value.strip()
    return None


def extract_from_cookie_text(cookie: str) -> dict[str, str]:
    cookie = cookie.strip()
    if not cookie:
        raise ValueError("cookie input is empty")
    return {
        "GW_MUSEAI_BASE_URL": "https://muse.ai",
        "GW_MUSEAI_COOKIE": cookie,
    }


def extract_from_header_text(text: str) -> dict[str, str]:
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    headers: dict[str, str] = {}
    for index in range(0, len(lines) - 1, 2):
        headers[lines[index].lower()] = lines[index + 1]

    cookie = headers.get("cookie")
    if not cookie:
        raise ValueError("header input contains no cookie")

    authority = headers.get(":authority", "muse.ai")
    scheme = headers.get(":scheme", "https")
    return {
        "GW_MUSEAI_BASE_URL": f"{scheme}://{authority}",
        "GW_MUSEAI_COOKIE": cookie,
    }


def extract_from_har(har: dict[str, Any]) -> dict[str, str]:
    entries = har.get("log", {}).get("entries", [])
    if not isinstance(entries, list):
        raise ValueError("HAR log.entries must be an array")

    result: dict[str, str] = {}
    for entry in entries:
        if not isinstance(entry, dict):
            continue
        request = entry.get("request", {})
        url = request.get("url")
        if not isinstance(url, str) or not url.startswith(("https://muse.ai/", "http://muse.ai/")):
            continue

        parsed = urlsplit(url)
        result.setdefault("GW_MUSEAI_BASE_URL", f"{parsed.scheme}://{parsed.netloc}")

        cookie = _header(request.get("headers", []), "cookie")
        if cookie:
            result.setdefault("GW_MUSEAI_COOKIE", cookie)

        payload = _response_json(entry)
        if payload is None:
            continue
        for key, value in _walk(payload):
            if not isinstance(value, str) or not value:
                continue
            key_lower = str(key).lower()
            if key_lower in _WS_URL_KEYS and value.startswith(("ws://", "wss://")):
                result.setdefault("GW_MUSEAI_WS_URL", value)
            elif key_lower == "access_token":
                result.setdefault("GW_MUSEAI_ACCESS_TOKEN", value)
            elif key_lower in ("ws_token", "websocket_token", "notary_token"):
                result.setdefault("GW_MUSEAI_NOTARY_TOKEN", value)
            elif key_lower == "token":
                result.setdefault("GW_MUSEAI_ACCESS_TOKEN", value)

    if not result:
        raise ValueError("HAR contains no Muse.ai requests")
    return result


def _env_value(value: str) -> str:
    if any(char.isspace() for char in value) or any(char in value for char in "#;\"'"):
        return json.dumps(value)
    return value


def update_env_file(path: str | Path, values: dict[str, str], replace: bool = False) -> None:
    env_path = Path(path)
    existing = env_path.read_text(encoding="utf-8") if env_path.exists() else ""
    lines = existing.splitlines()
    positions: dict[str, int] = {}

    for index, line in enumerate(lines):
        stripped = line.strip()
        if not stripped or stripped.startswith("#") or "=" not in stripped:
            continue
        key = stripped.split("=", 1)[0].strip()
        if key and key not in positions:
            positions[key] = index

    for key, value in values.items():
        line = f"{key}={_env_value(value)}"
        if key in positions:
            if replace:
                lines[positions[key]] = line
        else:
            lines.append(line)

    env_path.write_text("\n".join(lines).rstrip("\n") + "\n", encoding="utf-8")


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Extract local Muse.ai config into an env file")
    parser.add_argument("--har", type=Path, help="Path to a Muse.ai HAR export")
    parser.add_argument("--cookie-file", type=Path, help="Path to a browser cookie text file")
    parser.add_argument("--header-file", type=Path, help="Path to a DevTools copied headers text file")
    parser.add_argument("--out", type=Path, default=Path(".env"), help="Env file to update")
    parser.add_argument("--replace", action="store_true", help="Replace existing Muse.ai entries")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    if not args.har and not args.cookie_file and not args.header_file:
        raise SystemExit("provide --har, --cookie-file, or --header-file")

    values: dict[str, str] = {}
    if args.har:
        with args.har.open("r", encoding="utf-8") as handle:
            values.update(extract_from_har(json.load(handle)))
    if args.cookie_file:
        values.update(extract_from_cookie_text(args.cookie_file.read_text(encoding="utf-8")))
    if args.header_file:
        values.update(extract_from_header_text(args.header_file.read_text(encoding="utf-8")))

    update_env_file(args.out, values, replace=args.replace)
    print(f"updated {args.out} with {len(values)} Muse.ai config keys")
    print("secret values were not printed")
    return 0


if __name__ == "__main__":
    main()
