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
    apply_accounts,
    apply_settings,
    init_schema,
    load_env_vars,
    parse_cli_args,
    resolve_database_path,
    safe_resolve_sqlite_path,
    seed_from_gateway_config,
    show_current_settings,
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
