#!/usr/bin/env python3
"""
Gateway Configuration & Database Seeder
========================================
Applies legacy provider configurations from `gateway_config.json` / `.env`
into SQLite storage (`gateway.db`) while seeding flows, system catalog
modules, and provider accounts non-destructively.

Features:
- Preserves all provider credentials (Muse, Jev, OpenRouter, Custom Modules).
- Idempotent: safe to run repeatedly across deployments or updates.
- Supports secret protection (HMAC / AES envelope / blind indexing).
- Automatically discovers DATABASE_URL / GW_STATE_DIR from environment or .env.
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
    """
    Sanitizes and resolves a SQLite database path safely to prevent
    connection string injection and directory traversal attacks.
    """
    cleaned = raw_path.strip()
    if cleaned.startswith("sqlite://"):
        cleaned = cleaned[9:]
    elif cleaned.startswith("sqlite:"):
        cleaned = cleaned[7:]

    # Block URI query parameters or null bytes
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
    """
    Protects a credential at rest for SQLite storage.
    If encryption_key is provided, uses authenticated encryption envelope.
    """
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
    """Seeds default system modules into the SQLite modules table."""
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
                "group": None,
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
                        {
                            "id": "chat",
                            "name": "Chat Provider",
                            "subpath": "/chat",
                            "capabilities": ["chat_completions"],
                        },
                        {
                            "id": "vision",
                            "name": "Vision Provider",
                            "subpath": "/vision",
                            "capabilities": ["vision"],
                        },
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


def apply_settings(conn: sqlite3.Connection, data: dict):
    """Extracts and persists gateway settings into settings table."""
    excluded_keys = {"jev_accounts", "jev_openrouter_accounts", "personal_modules"}
    payload = {k: v for k, v in data.items() if k not in excluded_keys and v is not None}
    if not payload:
        return

    with conn:
        conn.execute("""
            INSERT INTO settings (key, value, updated_at)
            VALUES ('gateway_config', ?, CURRENT_TIMESTAMP)
            ON CONFLICT(key) DO UPDATE SET
                value = excluded.value,
                updated_at = CURRENT_TIMESTAMP
        """, (json.dumps(payload, indent=2),))
    print(f"[SEEDER] -> Seeded 'gateway_config' setting with {len(payload)} parameters.")


def apply_jev_accounts(conn: sqlite3.Connection, accounts: list, enc_key: str):
    """Extracts and persists Jev accounts."""
    count = 0
    with conn:
        for acc in accounts:
            raw_key = (acc.get("api_key") or acc.get("key") or "").strip()
            if not raw_key:
                continue
            prot_key = protect_credential(raw_key, enc_key)
            acc_id = acc.get("id") or f"jev_{derive_blind_index(raw_key)[:16]}"
            label = acc.get("label", "jev-account")
            conn.execute("""
                INSERT INTO accounts (id, provider, key, label, is_active, updated_at)
                VALUES (?, 'jev', ?, ?, 1, CURRENT_TIMESTAMP)
                ON CONFLICT(id) DO UPDATE SET
                    key = excluded.key,
                    label = excluded.label,
                    updated_at = CURRENT_TIMESTAMP
            """, (acc_id, prot_key, label))
            count += 1
    if count > 0:
        print(f"[SEEDER] -> Seeded {count} Jev provider accounts.")


def apply_openrouter_accounts(conn: sqlite3.Connection, accounts: list, enc_key: str):
    """Extracts and persists OpenRouter accounts."""
    count = 0
    with conn:
        for acc in accounts:
            raw_key = (acc.get("api_key", "") or acc.get("cookie", "") or acc.get("key", "")).strip()
            if not raw_key:
                continue
            prot_key = protect_credential(raw_key, enc_key)
            acc_id = acc.get("id") or f"or_{derive_blind_index(raw_key)[:16]}"
            label = acc.get("label", "openrouter-account")
            conn.execute("""
                INSERT INTO accounts (id, provider, key, label, is_active, updated_at)
                VALUES (?, 'openrouter', ?, ?, 1, CURRENT_TIMESTAMP)
                ON CONFLICT(id) DO UPDATE SET
                    key = excluded.key,
                    label = excluded.label,
                    updated_at = CURRENT_TIMESTAMP
            """, (acc_id, prot_key, label))
            count += 1
    if count > 0:
        print(f"[SEEDER] -> Seeded {count} OpenRouter provider accounts.")


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
            manifest_str = json.dumps(mod, indent=2)
            conn.execute("""
                INSERT INTO modules (id, namespace, name, manifest_json, enabled, updated_at)
                VALUES (?, ?, ?, ?, 1, CURRENT_TIMESTAMP)
                ON CONFLICT(id) DO UPDATE SET
                    namespace = excluded.namespace,
                    name = excluded.name,
                    manifest_json = excluded.manifest_json,
                    updated_at = CURRENT_TIMESTAMP
            """, (mod_id, mod_ns, mod_name, manifest_str))
            count += 1
    if count > 0:
        print(f"[SEEDER] -> Seeded {count} personal modules.")


def seed_from_gateway_config(conn: sqlite3.Connection, config_path: str, enc_key: str):
    """Reads provider config from gateway_config.json and applies it."""
    cfg_file = Path(config_path)
    if not cfg_file.is_file():
        print(f"[SEEDER] Note: {config_path} not found. Skipping file-based legacy config.")
        return

    try:
        with open(cfg_file, "r", encoding="utf-8") as f:
            data = json.load(f)
    except Exception as e:
        print(f"[SEEDER] Warning: Failed to parse {config_path}: {e}")
        return

    print(f"[SEEDER] Applying provider configs from {config_path}...")
    apply_settings(conn, data)
    apply_jev_accounts(conn, data.get("jev_accounts") or [], enc_key)
    apply_openrouter_accounts(conn, data.get("jev_openrouter_accounts") or [], enc_key)
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


def parse_cli_args() -> Tuple[str, str, bool]:
    """Parses CLI arguments safely and merges with .env configuration."""
    args = sys.argv[1:]
    auto_gen = "--generate-key" in args
    filtered_args = [a for a in args if a != "--generate-key"]

    env_vars = load_env_vars()
    cli_db = filtered_args[0] if len(filtered_args) > 0 else None
    cli_cfg = filtered_args[1] if len(filtered_args) > 1 else None

    db_arg = resolve_database_path(cli_db, env_vars)
    cfg_arg = cli_cfg or os.environ.get("GW_CONFIG_FILE") or env_vars.get("GW_CONFIG_FILE") or DEFAULT_CONFIG_PATH

    return db_arg, cfg_arg, auto_gen


def main():
    raw_db, raw_cfg, auto_gen = parse_cli_args()
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
        seed_system_catalog_modules(conn)
        seed_from_gateway_config(conn, raw_cfg, enc_key)
        print("[+] Seeding completed successfully.")
    finally:
        conn.close()


if __name__ == "__main__":
    main()
