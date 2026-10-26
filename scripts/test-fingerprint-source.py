#!/usr/bin/env python3
"""Regression checks for source receipts; never starts a BitNet process."""
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('fingerprint_source', Path(__file__).with_name('fingerprint-source.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class FingerprintTests(unittest.TestCase):
    def setUp(self):
        parent = Path(__file__).resolve().parent.parent / '.local/fingerprint-tests'
        parent.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(dir=parent)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / 'Cargo.toml').write_text('[workspace]\n')
        (self.root / 'Cargo.lock').write_text('version = 4\n')
        (self.root / 'crates/gui/src').mkdir(parents=True)
        (self.root / 'crates/gui/src/main.rs').write_text('fn main() {}\n')

    def test_generated_preview_is_not_an_application_input(self):
        before = module.fingerprint(self.root)
        for name in ('.local/preview/Cargo.toml', 'target/debug/scratch.rs', '__pycache__/cache.pyc'):
            path = self.root / 'crates/gui' / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('temporary data\n')
        self.assertEqual(module.fingerprint(self.root), before)

    def test_source_and_build_configuration_changes_are_detected(self):
        before = module.fingerprint(self.root)
        path = self.root / '.cargo/config.toml'
        path.parent.mkdir()
        path.write_text('[build]\njobs = 1\n')
        configured = module.fingerprint(self.root)
        self.assertNotEqual(before, configured)
        (self.root / 'crates/gui/src/main.rs').write_text('fn main() { println!("changed"); }\n')
        self.assertNotEqual(configured, module.fingerprint(self.root))

    def test_path_order_uses_portable_relative_strings(self):
        (self.root / 'crates/gui/Z.rs').write_text('z\n')
        (self.root / 'crates/gui/a.rs').write_text('a\n')
        names = ['Cargo.lock', 'Cargo.toml', 'crates/gui/Z.rs', 'crates/gui/a.rs', 'crates/gui/src/main.rs']
        expected = hashlib.sha256()
        for name in names:
            expected.update(name.encode() + b'\0')
            expected.update(hashlib.sha256((self.root / name).read_bytes()).digest())
        self.assertEqual(module.fingerprint(self.root)['source_sha256'], expected.hexdigest())


if __name__ == '__main__':
    unittest.main()
