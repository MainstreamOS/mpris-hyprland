import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("policies", Path(__file__).parents[1] / "packaging/manage-policies.py")
policies = importlib.util.module_from_spec(spec)
spec.loader.exec_module(policies)

class PolicyTests(unittest.TestCase):
    def setUp(self):
        self.directory=tempfile.TemporaryDirectory()
        self.root=Path(self.directory.name)
        self.path=self.root/policies.PATHS[0]
        self.path.parent.mkdir(parents=True)
    def tearDown(self): self.directory.cleanup()
    def write(self,data): self.path.write_text(json.dumps(data))
    def test_existing_admin_policies_and_extensions_survive_install_remove(self):
        original={"policies":{"Homepage":{"URL":"https://example.com"},"ExtensionSettings":{"other@addon":{"installation_mode":"blocked"}}},"custom":True}
        self.write(original);policies.update(self.root)
        self.assertIn(policies.EXTENSION_ID,json.loads(self.path.read_text())["policies"]["ExtensionSettings"])
        policies.update(self.root,True);self.assertEqual(json.loads(self.path.read_text()),original)
    def test_admin_override_survives(self):
        original={"policies":{"ExtensionSettings":{policies.EXTENSION_ID:{"installation_mode":"blocked"}}}}
        self.write(original);policies.update(self.root);policies.update(self.root,True)
        self.assertEqual(json.loads(self.path.read_text()),original)
    def test_invalid_json_is_not_overwritten(self):
        self.path.write_text('{broken');policies.update(self.root);self.assertEqual(self.path.read_text(),'{broken')
    def test_upgrade_restores_admin_policy_removed_by_old_package(self):
        backup=self.root/'var/lib/mpris-hyprland/firefox-upgrade-policy.json';backup.parent.mkdir(parents=True)
        backup.write_text('{"policies":{"DisableTelemetry":true}}');policies.update(self.root)
        self.assertTrue(json.loads(self.path.read_text())["policies"]["DisableTelemetry"]);self.assertFalse(backup.exists())
    def test_new_zen_policy_keeps_the_browser_baseline(self):
        policies.update(self.root);zen=json.loads((self.root/policies.PATHS[1]).read_text())["policies"]
        self.assertTrue(zen["DisableAppUpdate"]);self.assertEqual(zen["DefaultSerialGuardSetting"],3)
        self.assertEqual(zen["ExtensionSettings"][policies.EXTENSION_ID],policies.ENTRY)
    def test_repeated_install_is_idempotent(self):
        policies.update(self.root);first=self.path.read_bytes();policies.update(self.root);self.assertEqual(self.path.read_bytes(),first)
    def test_remove_cleans_up_file_with_only_our_entry(self):
        policies.update(self.root);policies.update(self.root,True);self.assertFalse(self.path.exists())

if __name__=='__main__':unittest.main()
