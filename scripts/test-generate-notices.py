#!/usr/bin/env python3
"""Offline fixture tests for license extraction; never touches Cargo's cache."""

import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


PROJECT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location("generate_notices", PROJECT / "scripts/generate-notices.py")
NOTICES = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(NOTICES)


class NoticeTests(unittest.TestCase):
    def setUp(self):
        local = PROJECT / ".local"
        local.mkdir(exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix="license-tests-", dir=local)
        self.root = Path(self.temporary.name).resolve()
        self.assertTrue(self.root.is_relative_to(local.resolve()))

    def tearDown(self):
        self.temporary.cleanup()

    def test_normal_and_build_closure_excludes_dev_only(self):
        metadata = {
            "workspace_members": ["app"],
            "resolve": {"nodes": [
                {"id": "app", "deps": [
                    {"pkg": "normal", "dep_kinds": [{"kind": None}]},
                    {"pkg": "build", "dep_kinds": [{"kind": "build"}]},
                    {"pkg": "dev", "dep_kinds": [{"kind": "dev"}]},
                ]},
                *({"id": name, "deps": []} for name in ("normal", "build", "dev")),
            ]},
        }
        self.assertEqual(NOTICES.selected_packages(metadata), {"app", "normal", "build"})

    def test_nested_documents_keep_original_characters_and_raw_hash(self):
        nested = self.root / "vendor/licenses"
        nested.mkdir(parents=True)
        path = nested / "MIT.txt"
        raw = "Copyright (c) 2026 原作者\r\nPermission follows.\r\n".encode("utf-8")
        path.write_bytes(raw)
        files, warnings = NOTICES.discover_documents(self.root, None)
        self.assertEqual(files, [path])
        self.assertEqual(warnings, [])
        documents = {}
        item = NOTICES.read_document(path, self.root, documents)
        self.assertEqual(item["sha256"], hashlib.sha256(raw).hexdigest())
        self.assertEqual(documents[item["sha256"]]["text"], raw.decode("utf-8"))
        NOTICES.read_document(path, self.root, documents)
        self.assertEqual(len(documents), 1)

    def test_outside_document_is_rejected(self):
        package = self.root / "package"
        package.mkdir()
        outside = self.root / "private.txt"
        outside.write_text("not a package document", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "outside"):
            NOTICES.read_document(outside, package, {})

    def test_empty_or_binary_document_is_rejected(self):
        path = self.root / "LICENSE"
        path.write_bytes(b"")
        with self.assertRaisesRegex(ValueError, "empty"):
            NOTICES.read_document(path, self.root, {})
        path.write_bytes(b"not\x00text")
        with self.assertRaisesRegex(ValueError, "NUL"):
            NOTICES.read_document(path, self.root, {})

    def test_bundled_sqlite_comment_keeps_source_hash_and_line_range(self):
        folder = self.root / "sqlite3"
        folder.mkdir()
        comment = b"/*\nOriginal copyright disclaimer.\n*/"
        source = comment + b"\n#define SOMETHING 1\n"
        (folder / "sqlite3.h").write_bytes(source)
        documents = {}
        item = NOTICES.embedded_source_notices("libsqlite3-sys", self.root, documents)[0]
        self.assertEqual(documents[item["sha256"]]["text"], comment.decode())
        self.assertEqual(item["provenance"]["source_sha256"], hashlib.sha256(source).hexdigest())
        self.assertEqual(item["provenance"]["line_end"], 3)
        self.assertEqual(item["provenance"]["byte_end_exclusive"], len(comment))

    def test_repository_root_mapping_requires_exact_commit_and_spdx(self):
        base = {"name": "root", "version": "1", "repository": "https://github.com/test/project", "spdx": "MIT", "vcs": {"commit": "a" * 40, "path_in_vcs": ""}, "license_files": [{"path": "LICENSE", "sha256": "document"}]}
        def child(name, commit="a" * 40, spdx="MIT"):
            return {**base, "name": name, "spdx": spdx, "vcs": {"commit": commit, "path_in_vcs": "child"}, "license_files": []}
        same, different, other_license = child("same"), child("different", "b" * 40), child("other", spdx="Apache-2.0")
        NOTICES.complete_root_licenses([base, same, different, other_license], {})
        self.assertEqual(same["license_files"][0]["provenance"]["kind"], "same-repository-commit-root-license")
        self.assertEqual(different["license_files"], [])
        self.assertEqual(other_license["license_files"], [])

    def test_recorded_upstream_text_requires_hash_and_exact_source(self):
        original_project = NOTICES.PROJECT
        NOTICES.PROJECT = self.root
        try:
            folder = self.root / "docs/licenses"
            folder.mkdir(parents=True)
            text = b"Copyright upstream\nOriginal permission text\n"
            (folder / "LICENSE.txt").write_bytes(text)
            entry = {"name": "example", "version": "1.0", "repository": "https://github.com/test/example", "spdx": "MIT", "vcs": {"commit": "c" * 40, "path_in_vcs": ""}, "license_files": []}
            provenance = {"name": "example", "version": "1.0", "repository": entry["repository"], "commit": "c" * 40, "url": "https://raw.githubusercontent.com/test/example/" + "c" * 40 + "/LICENSE", "file": "LICENSE.txt", "sha256": "0" * 64}
            manifest = folder / "upstream-provenance.json"
            manifest.write_text(json.dumps([provenance]), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "SHA256 mismatch"):
                NOTICES.complete_root_licenses([entry], {})
            provenance["sha256"] = hashlib.sha256(text).hexdigest()
            manifest.write_text(json.dumps([provenance]), encoding="utf-8")
            NOTICES.complete_root_licenses([entry], {})
            self.assertEqual(entry["license_files"][0]["provenance"]["commit"], "c" * 40)
        finally:
            NOTICES.PROJECT = original_project


if __name__ == "__main__":
    unittest.main()
