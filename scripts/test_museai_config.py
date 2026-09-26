import unittest
import os
import json
import tempfile
import sys
from pathlib import Path

# Add scripts directory to sys.path so we can import museai_config
sys.path.insert(0, str(Path(__file__).parent))

from museai_config import (
    extract_from_cookie_text,
    extract_from_har,
    extract_from_header_text,
    update_env_file,
)

class TestMuseAiConfig(unittest.TestCase):
    def test_extract_from_header_text(self):
        header_text = """
:authority
muse.ai
:method
GET
:path
/api/session
cookie
datr=my_cookie_val; hatch_sess=test_sess
user-agent
Mozilla/5.0
        """
        cfg = extract_from_header_text(header_text)
        self.assertEqual(cfg.get("GW_MUSEAI_COOKIE"), "datr=my_cookie_val; hatch_sess=test_sess")
        self.assertEqual(cfg.get("GW_MUSEAI_BASE_URL"), "https://muse.ai")

    def test_extract_from_cookie_text(self):
        cookie = "datr=test1; hatch_sess=test_sess; theme=dark"
        cfg = extract_from_cookie_text(cookie)
        self.assertEqual(cfg.get("GW_MUSEAI_COOKIE"), cookie.strip())
        self.assertEqual(cfg.get("GW_MUSEAI_BASE_URL"), "https://muse.ai")

    def test_extract_from_har(self):
        har_data = {
            "log": {
                "entries": [
                    {
                        "request": {
                            "url": "https://muse.ai/api/session",
                            "method": "GET",
                            "headers": [
                                {"name": "Cookie", "value": "hatch_sess=abc1234; ps_l=0"}
                            ]
                        },
                        "response": {
                            "status": 200,
                            "content": {
                                "text": json.dumps({"ok": True, "access_token": "token_abc_123"})
                            }
                        }
                    },
                    {
                        "request": {
                            "url": "https://muse.ai/api/hatch/vm/wake",
                            "method": "POST",
                            "headers": []
                        },
                        "response": {
                            "status": 200,
                            "content": {
                                "text": json.dumps({
                                    "status": "assigned",
                                    "vm_id": "vm-9999",
                                    "endpoint_url": "wss://vm-9999.metaaivm.com/"
                                })
                            }
                        }
                    }
                ]
            }
        }
        cfg = extract_from_har(har_data)
        self.assertEqual(cfg.get("GW_MUSEAI_COOKIE"), "hatch_sess=abc1234; ps_l=0")
        self.assertEqual(cfg.get("GW_MUSEAI_ACCESS_TOKEN"), "token_abc_123")
        self.assertEqual(cfg.get("GW_MUSEAI_WS_URL"), "wss://vm-9999.metaaivm.com/")
        self.assertEqual(cfg.get("GW_MUSEAI_BASE_URL"), "https://muse.ai")

    def test_update_env_file_preserves_and_replaces(self):
        """Test that update_env_file preserves existing values when replace=False."""
        with tempfile.NamedTemporaryFile("w+", delete=False) as f:
            f.write("EXISTING_KEY=old_val\nGW_MUSEAI_BASE_URL=https://old.url\n")
            env_path = f.name

        try:
            new_vars = {
                "GW_MUSEAI_BASE_URL": "https://muse.ai",
                "GW_MUSEAI_COOKIE": "session=123"
            }
            update_env_file(env_path, new_vars, replace=False)

            with open(env_path, "r") as f:
                content = f.read()

            self.assertIn("EXISTING_KEY=old_val", content)
            self.assertIn("GW_MUSEAI_BASE_URL=https://old.url", content)
            self.assertIn("GW_MUSEAI_COOKIE=session=123", content)
            self.assertNotIn("https://muse.ai", content)
        finally:
            if os.path.exists(env_path):
                os.remove(env_path)

        with tempfile.NamedTemporaryFile("w+", delete=False) as f:
            f.write("EXISTING_KEY=old_val\nGW_MUSEAI_BASE_URL=https://old.url\n")
            env_path = f.name

        try:
            new_vars = {
                "GW_MUSEAI_BASE_URL": "https://muse.ai",
                "GW_MUSEAI_COOKIE": "session=123"
            }
            update_env_file(env_path, new_vars, replace=True)

            with open(env_path, "r") as f:
                content = f.read()

            self.assertIn("EXISTING_KEY=old_val", content)
            self.assertIn("GW_MUSEAI_BASE_URL=https://muse.ai", content)
            self.assertIn("GW_MUSEAI_COOKIE=session=123", content)
            self.assertNotIn("https://old.url", content)
        finally:
            if os.path.exists(env_path):
                os.remove(env_path)

    def test_cli_stdout_secret_free(self):
        """Test that main() output does not contain secret values."""
        import io
        from contextlib import redirect_stdout
        from museai_config import main

        with tempfile.NamedTemporaryFile("w+", delete=False) as cf, \
             tempfile.NamedTemporaryFile("w+", delete=False) as ef:
            cf.write("datr=super_secret_cookie; hatch_sess=another_secret")
            cf_path = cf.name
            ef_path = ef.name

        try:
            buf = io.StringIO()
            with redirect_stdout(buf):
                exit_code = main(["--cookie-file", cf_path, "--out", ef_path])

            self.assertEqual(exit_code, 0)
            stdout = buf.getvalue()
            self.assertNotIn("super_secret_cookie", stdout)
            self.assertNotIn("another_secret", stdout)
            self.assertIn("secret values were not printed", stdout)
        finally:
            if os.path.exists(cf_path):
                os.remove(cf_path)
            if os.path.exists(ef_path):
                os.remove(ef_path)

if __name__ == "__main__":
    unittest.main()
