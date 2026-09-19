import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('profile_setup', Path(__file__).with_name('setup.py'))
setup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(setup)


class SetupTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.pi = self.root / 'pi'
        self.bin = self.root / 'bin'

    def test_plan_has_no_side_effects(self):
        files, pending = setup.install(self.pi, self.bin)
        self.assertEqual(len(files), 7)
        self.assertEqual(files, pending)
        self.assertEqual(list(self.root.iterdir()), [])

    def test_install_complete_and_idempotent(self):
        files, pending = setup.install(self.pi, self.bin, True)
        self.assertEqual(len(pending), 7)
        for source, target in files:
            self.assertEqual(source.read_bytes(), target.read_bytes())
        self.assertTrue((self.bin / 'herdr-name').stat().st_mode & 0o100)
        self.assertEqual(setup.install(self.pi, self.bin, True)[1], [])

    def test_late_conflict_prevents_all_writes(self):
        target = self.pi / 'extensions/herdr-room/transport.mjs'
        target.parent.mkdir(parents=True)
        target.write_text('personal modification')
        with self.assertRaisesRegex(ValueError, 'differing'):
            setup.install(self.pi, self.bin, True)
        self.assertFalse(self.bin.exists())
        self.assertFalse((self.pi / 'extensions/name-sync.ts').exists())
        self.assertEqual(target.read_text(), 'personal modification')

    def test_other_configuration_untouched(self):
        self.pi.mkdir()
        settings = self.pi / 'settings.json'
        settings.write_text('{"personal":"keep"}')
        setup.install(self.pi, self.bin, True)
        self.assertEqual(settings.read_text(), '{"personal":"keep"}')
        self.assertFalse((self.pi / 'auth.json').exists())

    def test_symlink_parent_refused(self):
        elsewhere = self.root / 'elsewhere'
        elsewhere.mkdir()
        self.pi.symlink_to(elsewhere, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'symlink'):
            setup.install(self.pi, self.bin, True)
        self.assertEqual(list(elsewhere.iterdir()), [])

    def test_broken_symlink_refused(self):
        self.pi.mkdir()
        extensions = self.pi / 'extensions'
        extensions.symlink_to(self.root / 'missing', target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'symlink'):
            setup.install(self.pi, self.bin, True)
        self.assertFalse(self.bin.exists())

    def test_non_directory_parent_refused(self):
        self.bin.write_text('keep')
        with self.assertRaisesRegex(ValueError, 'not a directory'):
            setup.install(self.pi, self.bin, True)
        self.assertFalse(self.pi.exists())
        self.assertEqual(self.bin.read_text(), 'keep')


if __name__ == '__main__':
    unittest.main()
