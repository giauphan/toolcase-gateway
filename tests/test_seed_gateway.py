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
    apply_jev_accounts,
    apply_openrouter_accounts,
    apply_settings,
    init_schema,
    load_env_vars,
    resolve_database_path,
    safe_resolve_sqlite_path,
    seed_from_gateway_config,
)


def test_load_env_vars(tmp_path: Path):
    env_file = tmp_path / ".env"
    env_file.write_text("DATABASE_URL=\"sqlite://.gw_state/gateway.db\"\nGW_STATE_DIR=.gw_state\n# comment\n")
    vars_dict = load_env_vars(env_file)
    assert vars_dict["DATABASE_URL"] == "sqlite://.gw_state/gateway.db"
    assert vars_dict["GW_STATE_DIR"] == ".gw_state"


def test_safe_resolve_sqlite_path(tmp_path: Path):
    target = tmp_path / "sub" / "gateway.db"
    resolved = safe_resolve_sqlite_path(f"sqlite://{target}")
    assert resolved == target.resolve()
    assert target.parent.is_dir()


def test_resolve_database_path():
    env_vars = {"DATABASE_URL": "sqlite://custom_state/app.db"}
    resolved = resolve_database_path(None, env_vars)
    assert resolved == "sqlite://custom_state/app.db"


def test_apply_settings_preserves_all_gateway_keys(tmp_path: Path):
    db_file = tmp_path / "test.db"
    conn = sqlite3.connect(str(db_file))
    init_schema(conn)

    data = {
        "target_host": "127.0.0.1",
        "target_port": 20128,
        "museai_ws_url": "wss://hatch.metaaivm.com/v1/noise",
        "museai_cookie": "test_cookie",
        "io_timeout_secs": 120,
        "default_video_prompt": "A beautiful sunset",
    }
    apply_settings(conn, data)

    cur = conn.cursor()
    cur.execute("SELECT value FROM settings WHERE key = 'gateway_config'")
    row = cur.fetchone()
    assert row is not None
    saved = json.loads(row[0])
    assert saved["target_host"] == "127.0.0.1"
    assert saved["museai_ws_url"] == "wss://hatch.metaaivm.com/v1/noise"
    assert saved["default_video_prompt"] == "A beautiful sunset"
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
    conn.close()
