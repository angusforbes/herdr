import json
import os
from pathlib import Path
import tempfile
import unittest
import tomllib
from unittest.mock import patch

import disposable


class ConfigurationTests(unittest.TestCase):
    def test_herdr_appearance_and_keys_copied_without_live_runtime_settings(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "config.toml"
            text = '''onboarding = true
[theme]
name = "terminal"
[theme.custom]
panel_bg = "black"
[keys]
prefix = "ctrl+space"
next_agent = ["shift+down", "prefix+down"]
[ui]
sidebar_max_width = 44
[ui.sidebar.agents]
rows = [["state_icon", {token = "$topic", italic = true}], ["$model", "$name"]]
[terminal]
default_shell = "/live/shell"
new_cwd = "follow"
[session]
resume_agents_on_restore = true
[plugins]
path = "/live/plugins"
'''
            source.write_text(text)
            copied = tomllib.loads(disposable.herdr_config(source))
            original = tomllib.loads(text)
            for key in ("theme", "keys", "ui"):
                self.assertEqual(copied[key], original[key])
            self.assertEqual(copied["terminal"]["default_shell"], "/bin/sh")
            self.assertFalse(copied["session"]["resume_agents_on_restore"])
            self.assertNotIn("plugins", copied)
            self.assertEqual(source.read_text(), text)

    def test_private_credentials_model_settings_and_only_two_local_hooks(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            source = base / "source"
            source.mkdir()
            (source / "settings.json").write_text(json.dumps({"extensions": ["!**"], "packages": ["npm:global-orchestrator"], "sessionDir": "/do/not/use", "defaultProvider": "test-provider", "defaultModel": "test-model", "defaultThinkingLevel": "low"}))
            original = b'{"test":"fake-test-value"}'
            (source / "auth.json").write_bytes(original)
            (source / "models.json").write_text("{}")
            preview = base / "preview"
            with patch.dict(os.environ, {"HERDR_SOCKET_PATH": "/live.sock", "HERDR_PANE_ID": "live", "HERDR_ENV": "1", "PI_SESSION_ID": "parent", "PI_CODING_AGENT_SESSION_DIR": "/live/sessions"}):
                env = disposable.environment(preview, source)
            self.assertNotEqual(env["HERDR_SOCKET_PATH"], "/live.sock")
            self.assertNotIn("HERDR_PANE_ID", env)
            self.assertNotIn("HERDR_ENV", env)
            self.assertNotIn("PI_SESSION_ID", env)
            target = Path(env["PI_CODING_AGENT_DIR"])
            self.assertEqual(target.stat().st_mode & 0o777, 0o700)
            self.assertEqual((target / "auth.json").stat().st_mode & 0o777, 0o600)
            self.assertFalse((target / "auth.json").is_symlink())
            self.assertEqual((target / "auth.json").read_bytes(), original)
            self.assertEqual(sorted(p.name for p in (target / "extensions").iterdir()), ["herdr-agent-state.ts", "herdr-room"])
            settings = json.loads((target / "settings.json").read_text())
            self.assertEqual(settings["defaultModel"], "test-model")
            self.assertEqual(settings["extensions"], [])
            self.assertEqual(settings["packages"], [])
            self.assertEqual(settings["defaultProjectTrust"], "never")
            self.assertNotIn("sessionDir", settings)
            (target / "auth.json").write_bytes(b"private-refreshed-test-value")
            disposable.environment(preview, source)
            self.assertEqual((target / "auth.json").read_bytes(), b"private-refreshed-test-value")
            self.assertEqual((source / "auth.json").read_bytes(), original)
            self.assertTrue(os.access(disposable.ROOT / "integrations/room/disposable.py", os.X_OK))


if __name__ == "__main__":
    unittest.main()
