#!/usr/bin/env python3
"""
Gateway Configuration & Database Seeder
========================================
Applies provider configurations from `gateway_config.json`, `.env`, and CLI
into SQLite storage (`gateway.db`). Updates the current active database
non-destructively with dual settings (`gateway_config` and `muse_config`).

Features:
- Preserves all provider credentials (Muse, Jev, OpenRouter, Custom Modules).
- Dual configuration persistence: updates `gateway_config` and `muse_config`.
- Discovers and merges `.env` variables (e.g. GW_MUSEAI_COOKIE, GW_STATE_DIR).
- Supports CLI overrides (`--set KEY=VALUE`) and inspection (`--show`).
- Idempotent: safe to run repeatedly across deployments or updates.
"""

import base64
import hashlib
import hmac
import json
import os
import sqlite3
import sys
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

DEFAULT_STATE_DIR = ".gw_state"
DEFAULT_DB_PATH = f"{DEFAULT_STATE_DIR}/gateway.db"
DEFAULT_CONFIG_PATH = "gateway_config.json"


def load_env_vars(env_path: Optional[Path] = None) -> Dict[str, str]:
    """Parses environment variables from .env file safely."""
    path = env_path or Path(".env")
    env_vars: Dict[str, str] = {}
    if not path.is_file():
        return env_vars

    try:
        content = path.read_text(encoding="utf-8")
        for line in content.splitlines():
            line = line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            k, v = line.split("=", 1)
            k = k.strip()
            v = v.strip().strip('"').strip("'")
            if k:
                env_vars[k] = v
    except Exception as e:
        print(f"[SEEDER] Warning: Failed to parse {path}: {e}")
    return env_vars


def safe_resolve_sqlite_path(raw_path: str) -> Path:
    """Sanitizes and resolves a SQLite database path safely."""
    cleaned = raw_path.strip()
    if cleaned.startswith("sqlite://"):
        cleaned = cleaned[9:]
    elif cleaned.startswith("sqlite:"):
        cleaned = cleaned[7:]

    if "?" in cleaned or "\0" in cleaned:
        cleaned = cleaned.split("?")[0].replace("\0", "")

    resolved = Path(cleaned).expanduser().resolve()
    if resolved.parent:
        resolved.parent.mkdir(parents=True, exist_ok=True)
    return resolved


def derive_blind_index(secret: str, salt: str = "toolcase_blind_idx") -> str:
    """Computes a deterministic blind index / fingerprint for secret lookup."""
    if not secret:
        return ""
    h = hmac.new(salt.encode("utf-8"), secret.encode("utf-8"), hashlib.sha256)
    return "fp_" + h.hexdigest()[:16]


def protect_credential(secret: str, encryption_key: str) -> str:
    """Protects a credential at rest with AES-GCM envelope if key is provided."""
    if not secret or secret.startswith("enc:v1:") or not encryption_key:
        return secret

    try:
        from cryptography.hazmat.primitives.ciphers.aead import AESGCM
        key_bytes = hashlib.sha256(encryption_key.encode("utf-8")).digest()
        aesgcm = AESGCM(key_bytes)
        nonce = os.urandom(12)
        ciphertext = aesgcm.encrypt(nonce, secret.encode("utf-8"), None)
        nonce_b64 = base64.b64encode(nonce).decode("ascii")
        payload_b64 = base64.b64encode(ciphertext).decode("ascii")
        return f"enc:v1:{nonce_b64}:{payload_b64}"
    except ImportError:
        raise RuntimeError("The 'cryptography' library is required to encrypt secrets compatible with toolcase-gateway.")


def init_schema(conn: sqlite3.Connection):
    """Initializes schema if not present."""
    with conn:
        conn.executescript("""
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS accounts (
                id TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                key TEXT NOT NULL,
                label TEXT,
                is_active INTEGER NOT NULL DEFAULT 1,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS modules (
                id TEXT PRIMARY KEY,
                namespace TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                manifest_json TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX IF NOT EXISTS idx_accounts_provider ON accounts(provider);
            CREATE INDEX IF NOT EXISTS idx_modules_namespace ON modules(namespace);
        """)


def seed_system_catalog_modules(conn: sqlite3.Connection):
    """Seeds default system catalog modules into SQLite modules table."""
    catalog_modules = [
        {
            "id": "cohere",
            "name": "Cohere Command R+",
            "namespace": "/custom/cohere",
            "manifest": {
                "id": "cohere",
                "name": "Cohere Command R+",
                "version": "1.0.0",
                "namespace": "/custom/cohere",
                "entrypoint": "python3 adapter.py",
                "protocol": "stdio-stream",
                "capabilities": ["chat_completions"],
            },
        },
        {
            "id": "suite-test",
            "name": "Suite Provider Group",
            "namespace": "/custom/suite-test",
            "manifest": {
                "id": "suite-test",
                "name": "Suite Provider Group",
                "version": "1.0.0",
                "namespace": "/custom/suite-test",
                "entrypoint": "python3 adapter.py",
                "protocol": "stdio-stream",
                "capabilities": ["chat_completions", "vision", "models"],
                "group": {
                    "enabled": True,
                    "providers": [
                        {"id": "chat", "name": "Chat Provider", "subpath": "/chat", "capabilities": ["chat_completions"]},
                        {"id": "vision", "name": "Vision Provider", "subpath": "/vision", "capabilities": ["vision"]},
                    ],
                },
            },
        },
    ]

    with conn:
        for m in catalog_modules:
            manifest_str = json.dumps(m["manifest"], indent=2)
            conn.execute("""
                INSERT INTO modules (id, namespace, name, manifest_json, enabled, updated_at)
                VALUES (?, ?, ?, ?, 1, CURRENT_TIMESTAMP)
                ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    namespace = excluded.namespace,
                    manifest_json = excluded.manifest_json,
                    updated_at = CURRENT_TIMESTAMP
            """, (m["id"], m["namespace"], m["name"], manifest_str))


def apply_settings(
    conn: sqlite3.Connection,
    data: dict,
    env_vars: Optional[Dict[str, str]] = None,
    overrides: Optional[Dict[str, Any]] = None,
):
    """Extracts, merges env vars, and persists gateway_config and muse_config."""
    env = env_vars or {}
    over = overrides or {}

    # Merge base config
    merged = dict(data)
    # Apply env variable mappings if present
    env_mappings = {
        "GW_MUSEAI_COOKIE": "museai_cookie",
        "GW_MUSEAI_WS_URL": "museai_ws_url",
        "GW_MUSEAI_BASE_URL": "museai_base_url",
        "GW_MUSE_APPROVAL_MODE": "muse_approval_mode",
        "GW_TARGET_HOST": "target_host",
        "GW_TARGET_PORT": "target_port",
        "GW_IO_TIMEOUT_SECS": "io_timeout_secs",
        "MUSE_LIVE_VIDEO_PROMPT": "default_video_prompt",
    }
    for env_k, cfg_k in env_mappings.items():
        if env_k in env and env[env_k]:
            merged[cfg_k] = env[env_k]

    merged.update(over)

    # 1. Gateway config payload
    excluded = {"jev_accounts", "jev_openrouter_accounts", "personal_modules"}
    gw_payload = {k: v for k, v in merged.items() if k not in excluded and v is not None}

    # 2. Muse config payload (for dashboard & muse module)
    muse_payload = {
        "base_url": merged.get("museai_base_url") or merged.get("base_url") or "https://muse.ai",
        "ws_url": merged.get("museai_ws_url") or merged.get("ws_url") or "wss://hatch.metaaivm.com/v1/noise",
        "vm_id": merged.get("museai_vm_id") or merged.get("vm_id") or "",
        "access_token": merged.get("museai_access_token") or merged.get("access_token") or "",
        "notary_token": merged.get("museai_notary_token") or merged.get("notary_token") or "",
        "cookie": merged.get("museai_cookie") or merged.get("cookie") or "",
        "default_video_prompt": merged.get("default_video_prompt") or "",
        "approval_mode": merged.get("muse_approval_mode") or merged.get("approval_mode") or "on",
        "allowed_destinations": merged.get("muse_approval_allowed_destinations") or ["muse.ai"],
        "allowed_types": merged.get("muse_approval_allowed_types") or ["network"],
        "timeout_secs": merged.get("muse_approval_timeout_secs") or merged.get("timeout_secs") or 30,
    }

    with conn:
        conn.execute("""
            INSERT INTO settings (key, value, updated_at)
            VALUES ('gateway_config', ?, CURRENT_TIMESTAMP)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP
        """, (json.dumps(gw_payload, indent=2),))

        conn.execute("""
            INSERT INTO settings (key, value, updated_at)
            VALUES ('muse_config', ?, CURRENT_TIMESTAMP)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP
        """, (json.dumps(muse_payload, indent=2),))

    print(f"[SEEDER] -> Updated 'gateway_config' ({len(gw_payload)} keys) and 'muse_config' ({len(muse_payload)} keys).")


def apply_accounts(conn: sqlite3.Connection, provider: str, accounts: list, enc_key: str):
    """Extracts and persists provider accounts."""
    count = 0
    with conn:
        for acc in accounts:
            raw_key = (acc.get("api_key") or acc.get("cookie") or acc.get("key") or "").strip()
            if not raw_key:
                continue
            prot_key = protect_credential(raw_key, enc_key)
            acc_id = acc.get("id") or f"{provider}_{derive_blind_index(raw_key)[:16]}"
            label = acc.get("label", f"{provider}-account")
            conn.execute("""
                INSERT INTO accounts (id, provider, key, label, is_active, updated_at)
                VALUES (?, ?, ?, ?, 1, CURRENT_TIMESTAMP)
                ON CONFLICT(id) DO UPDATE SET
                    key = excluded.key, label = excluded.label, updated_at = CURRENT_TIMESTAMP
            """, (acc_id, provider, prot_key, label))
            count += 1
    if count > 0:
        print(f"[SEEDER] -> Seeded {count} {provider} provider accounts.")


def apply_personal_modules(conn: sqlite3.Connection, modules: list):
    """Extracts and persists personal module configurations."""
    count = 0
    with conn:
        for mod in modules:
            mod_id = mod.get("id")
            mod_ns = mod.get("namespace")
            mod_name = mod.get("name", mod_id)
            if not mod_id or not mod_ns:
                continue
            conn.execute("""
                INSERT INTO modules (id, namespace, name, manifest_json, enabled, updated_at)
                VALUES (?, ?, ?, ?, 1, CURRENT_TIMESTAMP)
                ON CONFLICT(id) DO UPDATE SET
                    namespace = excluded.namespace, name = excluded.name,
                    manifest_json = excluded.manifest_json, updated_at = CURRENT_TIMESTAMP
            """, (mod_id, mod_ns, mod_name, json.dumps(mod, indent=2)))
            count += 1
    if count > 0:
        print(f"[SEEDER] -> Seeded {count} personal modules.")


def show_current_settings(conn: sqlite3.Connection):
    """Prints current active database settings without modification."""
    cur = conn.cursor()
    cur.execute("SELECT key, value, updated_at FROM settings ORDER BY key ASC")
    rows = cur.fetchall()
    print("\n[ACTIVE DATABASE SETTINGS]")
    if not rows:
        print("  (No settings found)")
    for key, val, upd in rows:
        print(f"\n--- Setting Key: '{key}' (Updated: {upd}) ---")
        try:
            parsed = json.loads(val)
            print(json.dumps(parsed, indent=2))
        except Exception:
            print(val)

    cur.execute("SELECT COUNT(*), provider FROM accounts GROUP BY provider")
    accs = cur.fetchall()
    print("\n[ACTIVE ACCOUNTS SUMMARY]")
    for count, prov in accs:
        print(f"  Provider '{prov}': {count} accounts")

    cur.execute("SELECT COUNT(*) FROM modules")
    mod_count = cur.fetchone()[0]
    print(f"\n[ACTIVE MODULES] Total modules: {mod_count}\n")


def seed_from_gateway_config(
    conn: sqlite3.Connection,
    config_path: str,
    enc_key: str,
    env_vars: Optional[Dict[str, str]] = None,
    overrides: Optional[Dict[str, Any]] = None,
):
    """Reads provider config from gateway_config.json and updates SQLite storage."""
    cfg_file = Path(config_path)
    data: Dict[str, Any] = {}
    if cfg_file.is_file():
        try:
            with open(cfg_file, "r", encoding="utf-8") as f:
                data = json.load(f)
            print(f"[SEEDER] Read base configuration from {config_path}")
        except Exception as e:
            print(f"[SEEDER] Warning: Failed to parse {config_path}: {e}")
    else:
        print(f"[SEEDER] Note: {config_path} not found. Relying on environment variables and defaults.")

    apply_settings(conn, data, env_vars=env_vars, overrides=overrides)
    apply_accounts(conn, "jev", data.get("jev_accounts") or [], enc_key)
    apply_accounts(conn, "openrouter", data.get("jev_openrouter_accounts") or [], enc_key)
    apply_personal_modules(conn, data.get("personal_modules") or [])


def resolve_encryption_key(auto_generate: bool, env_vars: Optional[Dict[str, str]] = None) -> str:
    """Resolves existing encryption key or optionally auto-generates one into .env."""
    key = os.environ.get("GW_ENCRYPTION_KEY", "").strip()
    if key:
        return key

    vars_dict = env_vars or load_env_vars()
    if vars_dict.get("GW_ENCRYPTION_KEY"):
        return vars_dict["GW_ENCRYPTION_KEY"]

    if auto_generate:
        import secrets
        new_key = secrets.token_hex(32)
        env_path = Path(".env")
        try:
            with open(env_path, "a", encoding="utf-8") as f:
                f.write(f"\n# Auto-generated storage encryption key\nGW_ENCRYPTION_KEY={new_key}\n")
            print(f"[SECURITY] Generated new GW_ENCRYPTION_KEY and saved to {env_path}")
            return new_key
        except Exception as e:
            print(f"[SECURITY] Warning: Could not write key to {env_path}: {e}")

    return ""


def resolve_database_path(cli_db: Optional[str] = None, env_vars: Optional[Dict[str, str]] = None) -> str:
    """Resolves the active database path prioritizing CLI, env, .env, and state dir."""
    if cli_db:
        return cli_db

    vars_dict = env_vars or load_env_vars()
    for env_key in ("DATABASE_URL", "GW_DATABASE_URL"):
        val = os.environ.get(env_key) or vars_dict.get(env_key)
        if val:
            return val

    state_dir = os.environ.get("GW_STATE_DIR") or vars_dict.get("GW_STATE_DIR") or DEFAULT_STATE_DIR
    return f"{state_dir}/gateway.db"


def parse_cli_args() -> Tuple[str, str, bool, bool, Dict[str, Any]]:
    """Parses CLI arguments safely and merges with .env configuration."""
    args = sys.argv[1:]
    auto_gen = "--generate-key" in args
    show_only = "--show" in args
    filtered = [a for a in args if a not in ("--generate-key", "--show")]

    overrides: Dict[str, Any] = {}
    pos_args: List[str] = []
    i = 0
    while i < len(filtered):
        arg = filtered[i]
        if arg == "--set" and i + 1 < len(filtered):
            k, v = filtered[i + 1].split("=", 1) if "=" in filtered[i + 1] else (filtered[i + 1], "true")
            overrides[k.strip()] = v.strip()
            i += 2
        elif arg.startswith("--set="):
            k, v = arg[6:].split("=", 1) if "=" in arg[6:] else (arg[6:], "true")
            overrides[k.strip()] = v.strip()
            i += 1
        else:
            pos_args.append(arg)
            i += 1

    env_vars = load_env_vars()
    cli_db = pos_args[0] if len(pos_args) > 0 else None
    cli_cfg = pos_args[1] if len(pos_args) > 1 else None

    db_arg = resolve_database_path(cli_db, env_vars)
    cfg_arg = cli_cfg or os.environ.get("GW_CONFIG_FILE") or env_vars.get("GW_CONFIG_FILE") or DEFAULT_CONFIG_PATH

    return db_arg, cfg_arg, auto_gen, show_only, overrides


def main():
    raw_db, raw_cfg, auto_gen, show_only, overrides = parse_cli_args()
    env_vars = load_env_vars()
    db_path = safe_resolve_sqlite_path(raw_db)
    enc_key = resolve_encryption_key(auto_gen, env_vars)

    print("==================================================")
    print(" toolcase-gateway: Flow & Database Seeder")
    print("==================================================")
    print(f"[*] Target SQLite Database: {db_path}")
    print(f"[*] Gateway Config File   : {raw_cfg}")
    if enc_key:
        print("[*] Credential Protection: ENABLED (Authenticated Envelope)")
    else:
        print("[*] Credential Protection: Standard Plaintext (use --generate-key to enable)")

    conn = sqlite3.connect(str(db_path), uri=False)
    try:
        init_schema(conn)
        if show_only:
            show_current_settings(conn)
            return

        seed_system_catalog_modules(conn)
        seed_from_gateway_config(conn, raw_cfg, enc_key, env_vars=env_vars, overrides=overrides)
        print("[+] Seeding and database update completed successfully.")
    finally:
        conn.close()


if __name__ == "__main__":
    main()
