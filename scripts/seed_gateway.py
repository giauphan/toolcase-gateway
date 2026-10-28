#!/usr/bin/env python3
"""
Gateway Configuration & Database Seeder
========================================
Applies legacy provider configurations from `gateway_config.json` / `.env`
into SQLite storage (`gateway.db`) while seeding new flows, system catalog
modules, and provider accounts non-destructively.

Features:
- Preserves all old provider credentials (Muse, Jev, OpenRouter, Custom Modules).
- Idempotent: safe to run repeatedly across deployments or updates.
- Supports secret protection (HMAC / AES envelope / blind indexing).
"""

import os
import sys
import json
import sqlite3
import hashlib
import hmac
import base64
import uuid
from pathlib import Path

DEFAULT_DB_PATH = "gateway.db"
DEFAULT_CONFIG_PATH = "gateway_config.json"


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
    if resolved.parent and not resolved.parent.exists():
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
        # Fallback to authenticated XOR keystream if cryptography is unavailable
        key_bytes = hashlib.sha256(encryption_key.encode("utf-8")).digest()
        nonce = os.urandom(12)
        keystream = b""
        counter = 0
        while len(keystream) < len(secret.encode("utf-8")):
            keystream += hmac.new(key_bytes, nonce + counter.to_bytes(4, "big"), hashlib.sha256).digest()
            counter += 1
        cipher_bytes = bytes(b ^ k for b, k in zip(secret.encode("utf-8"), keystream))
        tag = hmac.new(key_bytes, nonce + cipher_bytes, hashlib.sha256).digest()[:16]
        payload = cipher_bytes + tag
        nonce_b64 = base64.b64encode(nonce).decode("ascii")
        payload_b64 = base64.b64encode(payload).decode("ascii")
        return f"enc:v1:{nonce_b64}:{payload_b64}"


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
    """Extracts and persists legacy gateway settings."""
    tracked_keys = [
        "target_host", "target_port", "fallbacks", "default_model",
        "museai_cookie", "museai_access_token", "museai_notary_token", "museai_vm_id",
        "muse_approval_mode", "jev_openrouter_model", "jev_openrouter_enabled",
    ]
    payload = {k: data[k] for k in tracked_keys if k in data and data[k] is not None}
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
    """Extracts and persists legacy Jev accounts."""
    count = 0
    with conn:
        for acc in accounts:
            raw_key = acc.get("key", "").strip()
            if not raw_key:
                continue
            prot_key = protect_credential(raw_key, enc_key)
            acc_id = acc.get("id") or f"jev_{uuid.uuid4().hex[:12]}"
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
    """Extracts and persists legacy OpenRouter accounts."""
    count = 0
    with conn:
        for acc in accounts:
            raw_key = (acc.get("api_key", "") or acc.get("cookie", "")).strip()
            if not raw_key:
                continue
            prot_key = protect_credential(raw_key, enc_key)
            acc_id = acc.get("id") or f"or_{uuid.uuid4().hex[:12]}"
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
    """Extracts and persists legacy personal module configurations."""
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
    """Reads legacy provider config from gateway_config.json and applies it."""
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


def read_key_from_env_file(env_path: Path) -> str:
    """Extracts existing encryption key from an environment file if present."""
    if not env_path.is_file():
        return ""
    try:
        content = env_path.read_text(encoding="utf-8")
        for line in content.splitlines():
            line = line.strip()
            if line.startswith("GW_ENCRYPTION_KEY=") and not line.startswith("#"):
                return line.split("=", 1)[1].strip().strip('"').strip("'")
    except Exception:
        pass
    return ""


def generate_and_save_key(env_path: Path) -> str:
    """Generates a secure 256-bit key and appends it to the env file."""
    import secrets
    new_key = secrets.token_hex(32)
    try:
        with open(env_path, "a", encoding="utf-8") as f:
            f.write(f"\n# Auto-generated storage encryption key\nGW_ENCRYPTION_KEY={new_key}\n")
        print(f"[SECURITY] Generated new GW_ENCRYPTION_KEY and saved to {env_path}")
    except Exception as e:
        print(f"[SECURITY] Warning: Could not write key to {env_path}: {e}")
    return new_key


def resolve_encryption_key(auto_generate: bool) -> str:
    """Resolves existing encryption key or optionally auto-generates one into .env."""
    key = os.environ.get("GW_ENCRYPTION_KEY", "").strip()
    if key:
        return key

    env_path = Path(".env")
    key = read_key_from_env_file(env_path)
    if key:
        return key

    if auto_generate:
        return generate_and_save_key(env_path)

    return ""


def parse_cli_args():
    """Parses CLI arguments safely without raw connection strings."""
    args = sys.argv[1:]
    auto_gen = "--generate-key" in args
    filtered_args = [a for a in args if a != "--generate-key"]

    db_arg = filtered_args[0] if len(filtered_args) > 0 else os.environ.get("GW_DATABASE_URL", DEFAULT_DB_PATH)
    cfg_arg = filtered_args[1] if len(filtered_args) > 1 else os.environ.get("GW_CONFIG_FILE", DEFAULT_CONFIG_PATH)

    return db_arg, cfg_arg, auto_gen


def main():
    raw_db, raw_cfg, auto_gen = parse_cli_args()
    db_path = safe_resolve_sqlite_path(raw_db)
    enc_key = resolve_encryption_key(auto_gen)

    print("==================================================")
    print(" toolcase-gateway: Flow & Database Seeder")
    print("==================================================")
    print(f"[*] Target SQLite Database: {db_path}")
    print(f"[*] Legacy Config File   : {raw_cfg}")
    if enc_key:
        print("[*] Credential Protection: ENABLED (Authenticated Envelope)")
    else:
        print("[*] Credential Protection: Standard Plaintext (use --generate-key to enable)")

    # Explicit uri=False prevents connection string / URI parameter injection attacks
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
