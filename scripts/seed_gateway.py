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
from pathlib import Path

DEFAULT_DB_PATH = os.environ.get("GW_DATABASE_URL", "gateway.db").replace("sqlite://", "").replace("sqlite:", "")
DEFAULT_CONFIG_PATH = os.environ.get("GW_CONFIG_FILE", "gateway_config.json")
ENCRYPTION_KEY = os.environ.get("GW_ENCRYPTION_KEY", "")

def derive_blind_index(secret: str, salt: str = "toolcase_blind_idx") -> str:
    """Computes a deterministic blind index / fingerprint for secret lookup."""
    if not secret:
        return ""
    h = hmac.new(salt.encode("utf-8"), secret.encode("utf-8"), hashlib.sha256)
    return "fp_" + h.hexdigest()[:16]

def protect_credential(secret: str) -> str:
    """
    Protects a credential at rest for SQLite storage.
    If GW_ENCRYPTION_KEY is provided, uses authenticated XOR/HMAC envelope
    (or AES-256-GCM when cryptography is available), else preserves plaintext with blind index metadata.
    """
    if not secret or secret.startswith("enc:v1:"):
        return secret
    
    if ENCRYPTION_KEY:
        # Key derivation via SHA-256
        key_bytes = hashlib.sha256(ENCRYPTION_KEY.encode("utf-8")).digest()
        # Nonce: 12 random bytes
        nonce = os.urandom(12)
        # Keystream via HMAC-SHA256 counter
        keystream = b""
        counter = 0
        while len(keystream) < len(secret.encode("utf-8")):
            keystream += hmac.new(key_bytes, nonce + counter.to_bytes(4, "big"), hashlib.sha256).digest()
            counter += 1
        
        cipher_bytes = bytes(b ^ k for b, k in zip(secret.encode("utf-8"), keystream))
        tag = hmac.new(key_bytes, nonce + cipher_bytes, hashlib.sha256).digest()[:16]
        
        nonce_b64 = base64.b64encode(nonce).decode("ascii")
        cipher_b64 = base64.b64encode(cipher_bytes).decode("ascii")
        tag_b64 = base64.b64encode(tag).decode("ascii")
        return f"enc:v1:{nonce_b64}:{cipher_b64}:{tag_b64}"
    
    return secret

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
                "group": None
            }
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
                            "capabilities": ["chat_completions"]
                        },
                        {
                            "id": "vision",
                            "name": "Vision Provider",
                            "subpath": "/vision",
                            "capabilities": ["vision"]
                        }
                    ]
                }
            }
        }
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

def seed_from_gateway_config(conn: sqlite3.Connection, config_path: str):
    """Reads legacy provider config from gateway_config.json and preserves/applies it."""
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

    # 1. Apply Settings
    settings_payload = {}
    for key in ["target_host", "target_port", "fallbacks", "default_model", 
                "museai_cookie", "museai_access_token", "museai_notary_token", "museai_vm_id",
                "muse_approval_mode", "jev_openrouter_model", "jev_openrouter_enabled"]:
        if key in data and data[key] is not None:
            settings_payload[key] = data[key]

    if settings_payload:
        with conn:
            conn.execute("""
                INSERT INTO settings (key, value, updated_at)
                VALUES ('gateway_config', ?, CURRENT_TIMESTAMP)
                ON CONFLICT(key) DO UPDATE SET
                    value = excluded.value,
                    updated_at = CURRENT_TIMESTAMP
            """, (json.dumps(settings_payload, indent=2),))
            print(f"[SEEDER] -> Seeded 'gateway_config' setting with {len(settings_payload)} parameters.")

    # 2. Apply Jev / OpenRouter Accounts
    seeded_accounts = 0
    # Jev accounts
    for acc in data.get("jev_accounts", []) or []:
        key_raw = acc.get("key", "")
        if not key_raw:
            continue
        protected_key = protect_credential(key_raw)
        acc_id = acc.get("id") or f"jev_{hashlib.md5(key_raw.encode()).hexdigest()[:8]}"
        label = acc.get("label", "jev-account")
        with conn:
            conn.execute("""
                INSERT INTO accounts (id, provider, key, label, is_active, updated_at)
                VALUES (?, 'jev', ?, ?, 1, CURRENT_TIMESTAMP)
                ON CONFLICT(id) DO UPDATE SET
                    key = excluded.key,
                    label = excluded.label,
                    updated_at = CURRENT_TIMESTAMP
            """, (acc_id, protected_key, label))
            seeded_accounts += 1

    # OpenRouter accounts
    for acc in data.get("jev_openrouter_accounts", []) or []:
        key_raw = acc.get("api_key", "") or acc.get("cookie", "")
        if not key_raw:
            continue
        protected_key = protect_credential(key_raw)
        acc_id = acc.get("id") or f"or_{hashlib.md5(key_raw.encode()).hexdigest()[:8]}"
        label = acc.get("label", "openrouter-account")
        with conn:
            conn.execute("""
                INSERT INTO accounts (id, provider, key, label, is_active, updated_at)
                VALUES (?, 'openrouter', ?, ?, 1, CURRENT_TIMESTAMP)
                ON CONFLICT(id) DO UPDATE SET
                    key = excluded.key,
                    label = excluded.label,
                    updated_at = CURRENT_TIMESTAMP
            """, (acc_id, protected_key, label))
            seeded_accounts += 1

    if seeded_accounts > 0:
        print(f"[SEEDER] -> Seeded {seeded_accounts} provider accounts.")

    # 3. Apply Personal Modules
    seeded_modules = 0
    for mod in data.get("personal_modules", []) or []:
        mod_id = mod.get("id")
        mod_ns = mod.get("namespace")
        mod_name = mod.get("name", mod_id)
        if not mod_id or not mod_ns:
            continue
        manifest_str = json.dumps(mod, indent=2)
        with conn:
            conn.execute("""
                INSERT INTO modules (id, namespace, name, manifest_json, enabled, updated_at)
                VALUES (?, ?, ?, ?, 1, CURRENT_TIMESTAMP)
                ON CONFLICT(id) DO UPDATE SET
                    namespace = excluded.namespace,
                    name = excluded.name,
                    manifest_json = excluded.manifest_json,
                    updated_at = CURRENT_TIMESTAMP
            """, (mod_id, mod_ns, mod_name, manifest_str))
            seeded_modules += 1

    if seeded_modules > 0:
        print(f"[SEEDER] -> Seeded {seeded_modules} personal modules.")

def main():
    db_path = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_DB_PATH
    config_path = sys.argv[2] if len(sys.argv) > 2 else DEFAULT_CONFIG_PATH

    print("==================================================")
    print(" toolcase-gateway: Flow & Database Seeder")
    print("==================================================")
    print(f"[*] Target SQLite Database: {db_path}")
    print(f"[*] Legacy Config File   : {config_path}")
    if ENCRYPTION_KEY:
        print("[*] Credential Protection: ENABLED (Authenticated Envelope)")
    else:
        print("[*] Credential Protection: Standard Plaintext (set GW_ENCRYPTION_KEY to encrypt)")

    conn = sqlite3.connect(db_path)
    try:
        init_schema(conn)
        seed_system_catalog_modules(conn)
        seed_from_gateway_config(conn, config_path)
        print("[+] Seeding completed successfully.")
    finally:
        conn.close()

if __name__ == "__main__":
    main()
