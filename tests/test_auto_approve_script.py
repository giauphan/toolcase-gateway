import unittest
import os
import re

class TestAutoApproveScript(unittest.TestCase):
    def test_html_has_primary_action_button(self):
        path = "target/html.html"
        if not os.path.exists(path):
            self.skipTest("target/html.html not present")
        with open(path, "r", encoding="utf-8") as f:
            html = f.read()
        
        self.assertIn('data-hatch-approval-primary-action="true"', html)
        self.assertIn('Allow', html)

    def test_html_no_popup_has_avatar(self):
        path = "target/html-no-popup-approve.html"
        if not os.path.exists(path):
            self.skipTest("target/html-no-popup-approve.html not present")
        with open(path, "r", encoding="utf-8") as f:
            html = f.read()

        self.assertIn('data-hatch-avatar-interaction="true"', html)
        self.assertIn('aria-label="Muse"', html)

    def test_script_logic_selectors(self):
        script_path = "scripts/museai_auto_approve.js"
        with open(script_path, "r", encoding="utf-8") as f:
            js = f.read()

        self.assertIn('data-hatch-approval-primary-action', js)
        self.assertIn('data-hatch-avatar-interaction', js)
        self.assertIn('collapsible', js.lower() or 'trigger' in js.lower())

if __name__ == "__main__":
    unittest.main()
