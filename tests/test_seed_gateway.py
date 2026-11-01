#!/usr/bin/env python3
import json
import sqlite3
import sys
import tempfile
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from scripts.seed_gateway import (
    ENCRYPTED_PREFIX,
    apply_accounts,
    apply_settings,
    init_schema,
    load_env_vars,
    parse_cli_args,
    protect_credential,
    resolve_database_path,
    safe_resolve_sqlite_path,
    seed_from_gateway_config,
    show_current_settings,
    unprotect_credential,
)


def test_load_env_vars(tmp_path: Path):
    env_file = tmp_path / ".env"
    env_file.write_text("DATABASE_URL=\"sqlite://.gw_state/gateway.db\"\nGW_STATE_DIR=.gw_state\nGW_MUSEAI_COOKIE=test_env_cookie\n# comment\n")
    vars_dict = load_env_vars(env_file)
    assert vars_dict["DATABASE_URL"] == "sqlite://.gw_state/gateway.db"
    assert vars_dict["GW_STATE_DIR"] == ".gw_state"
    assert vars_dict["GW_MUSEAI_COOKIE"] == "test_env_cookie"


def test_safe_resolve_sqlite_path(tmp_path: Path):
    target = tmp_path / "sub" / "gateway.db"
    resolved = safe_resolve_sqlite_path(f"sqlite://{target}")
    assert resolved == target.resolve()
    assert target.parent.is_dir()


def test_resolve_database_path():
    env_vars = {"DATABASE_URL": "sqlite://custom_state/app.db"}
    resolved = resolve_database_path(None, env_vars)
    assert resolved == "sqlite://custom_state/app.db"


def test_apply_settings_dual_persistence_and_env_merge(tmp_path: Path):
    db_file = tmp_path / "test.db"
    conn = sqlite3.connect(str(db_file))
    init_schema(conn)

    data = {
        "target_host": "127.0.0.1",
        "target_port": 20128,
        "default_video_prompt": "A beautiful sunset",
    }
    env_vars = {
        "GW_MUSEAI_COOKIE": "env_cookie_value",
        "GW_MUSEAI_WS_URL": "wss://hatch.metaaivm.com/v1/noise",
    }
    overrides = {
        "muse_approval_mode": "observe",
    }

    apply_settings(conn, data, env_vars=env_vars, overrides=overrides)

    cur = conn.cursor()
    # Check gateway_config
    cur.execute("SELECT value FROM settings WHERE key = 'gateway_config'")
    row_gw = cur.fetchone()
    assert row_gw is not None
    saved_gw = json.loads(row_gw[0])
    assert saved_gw["target_host"] == "127.0.0.1"
    assert saved_gw["museai_cookie"] == "env_cookie_value"
    assert saved_gw["muse_approval_mode"] == "observe"

    # Check muse_config
    cur.execute("SELECT value FROM settings WHERE key = 'muse_config'")
    row_muse = cur.fetchone()
    assert row_muse is not None
    saved_muse = json.loads(row_muse[0])
    assert saved_muse["cookie"] == "env_cookie_value"
    assert saved_muse["ws_url"] == "wss://hatch.metaaivm.com/v1/noise"
    assert saved_muse["approval_mode"] == "observe"
    assert saved_muse["default_video_prompt"] == "A beautiful sunset"

    conn.close()


def test_seed_from_gateway_config_end_to_end(tmp_path: Path):
    db_file = tmp_path / "test_gw.db"
    cfg_file = tmp_path / "gateway_config.json"
    cfg_data = {
        "target_host": "127.0.0.1",
        "target_port": 20128,
        "jev_accounts": [
            {"api_key": "sk-jev-12345", "label": "test-jev"}
        ],
        "jev_openrouter_accounts": [
            {"api_key": "sk-or-67890", "label": "test-or"}
        ],
        "museai_cookie": "dummy_cookie",
    }
    cfg_file.write_text(json.dumps(cfg_data))

    conn = sqlite3.connect(str(db_file))
    init_schema(conn)
    seed_from_gateway_config(conn, str(cfg_file), enc_key="")

    cur = conn.cursor()
    cur.execute("SELECT COUNT(*) FROM accounts")
    acc_count = cur.fetchone()[0]
    assert acc_count == 2

    # Verify both settings exist
    cur.execute("SELECT key FROM settings ORDER BY key ASC")
    keys = [r[0] for r in cur.fetchall()]
    assert "gateway_config" in keys
    assert "muse_config" in keys
    conn.close()


def test_show_current_settings_runs_without_error(tmp_path: Path, capsys):
    db_file = tmp_path / "test_show.db"
    conn = sqlite3.connect(str(db_file))
    init_schema(conn)
    apply_settings(conn, {"target_port": 28130})
    show_current_settings(conn)
    captured = capsys.readouterr()
    assert "[ACTIVE DATABASE SETTINGS]" in captured.out
    assert "target_port" in captured.out
    conn.close()


def test_apply_accounts_idempotency_and_uuid_generation(tmp_path: Path):
    db_file = tmp_path / "test_accounts.db"
    conn = sqlite3.connect(str(db_file))
    init_schema(conn)

    # 1. Accounts without explicit ID should receive generated provider prefix IDs
    accounts = [
        {"key": "sk-jev-sample-key", "label": "Initial Label"},
        {"id": "explicit-id-123", "key": "sk-jev-explicit-key", "label": "Explicit Label"},
    ]
    apply_accounts(conn, "jev", accounts, enc_key="")

    cur = conn.cursor()
    cur.execute("SELECT id, key, label FROM accounts ORDER BY label ASC")
    rows = cur.fetchall()
    assert len(rows) == 2
    explicit_row = next(r for r in rows if r[0] == "explicit-id-123")
    assert explicit_row[2] == "Explicit Label"

    auto_row = next(r for r in rows if r[0] != "explicit-id-123")
    assert auto_row[0].startswith("jev_")
    generated_id = auto_row[0]

    # 2. Re-applying the same accounts with an updated label should update existing row without duplicates
    updated_accounts = [
        {"key": "sk-jev-sample-key", "label": "Updated Auto Label"},
        {"id": "explicit-id-123", "key": "sk-jev-explicit-key", "label": "Updated Explicit Label"},
    ]
    apply_accounts(conn, "jev", updated_accounts, enc_key="")

    cur.execute("SELECT id, key, label FROM accounts ORDER BY label ASC")
    updated_rows = cur.fetchall()
    assert len(updated_rows) == 2
    reused_auto_row = next(r for r in updated_rows if r[1] == "sk-jev-sample-key")
    assert reused_auto_row[0] == generated_id
    assert reused_auto_row[2] == "Updated Auto Label"

    conn.close()


def test_apply_accounts_idempotency_with_encryption_key(tmp_path: Path):
    db_file = tmp_path / "test_enc_accounts.db"
    conn = sqlite3.connect(str(db_file))
    init_schema(conn)

    enc_key = "test_super_secret_master_key_12345"
    accounts = [
        {"key": "sk-jev-confidential-key", "label": "Encrypted Auto Account"},
        {"id": "enc-explicit-1", "key": "sk-jev-second-key", "label": "Encrypted Explicit Account"},
    ]

    # First seeding with non-empty encryption key
    apply_accounts(conn, "jev", accounts, enc_key=enc_key)

    cur = conn.cursor()
    cur.execute("SELECT id, key, label FROM accounts ORDER BY label ASC")
    rows = cur.fetchall()
    assert len(rows) == 2
    auto_id = None
    for acc_id, enc_value, label in rows:
        assert enc_value.startswith(ENCRYPTED_PREFIX)
        decrypted = unprotect_credential(enc_value, enc_key)
        if acc_id == "enc-explicit-1":
            assert decrypted == "sk-jev-second-key"
        else:
            assert decrypted == "sk-jev-confidential-key"
            auto_id = acc_id

    assert auto_id is not None

    # Second seeding with the same encryption key - must not insert duplicate rows
    updated_accounts = [
        {"key": "sk-jev-confidential-key", "label": "Updated Encrypted Auto Account"},
        {"id": "enc-explicit-1", "key": "sk-jev-second-key", "label": "Updated Encrypted Explicit Account"},
    ]
    apply_accounts(conn, "jev", updated_accounts, enc_key=enc_key)

    cur.execute("SELECT id, key, label FROM accounts ORDER BY label ASC")
    updated_rows = cur.fetchall()
    assert len(updated_rows) == 2
    reused_auto = next(r for r in updated_rows if r[0] == auto_id)
    assert reused_auto[2] == "Updated Encrypted Auto Account"
    assert unprotect_credential(reused_auto[1], enc_key) == "sk-jev-confidential-key"

    # Third seeding supplying the already-encrypted string without explicit ID
    stored_enc_key = reused_auto[1]
    enc_input_accounts = [
        {"key": stored_enc_key, "label": "Preserved Encrypted Input Account"},
    ]
    apply_accounts(conn, "jev", enc_input_accounts, enc_key=enc_key)
    cur.execute("SELECT id, key, label FROM accounts WHERE provider = 'jev'")
    final_rows = cur.fetchall()
    assert len(final_rows) == 2
    matched_by_enc_input = next(r for r in final_rows if r[0] == auto_id)
    assert matched_by_enc_input[2] == "Preserved Encrypted Input Account"

    conn.close()
