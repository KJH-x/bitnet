#!/usr/bin/env python3
"""Fingerprint application build inputs; fail if they changed during a release build."""
import argparse
import hashlib
import json
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument('--write', type=Path)
p.add_argument('--check', type=Path)
args = p.parse_args()
root = Path(__file__).resolve().parent.parent
paths = [root / 'Cargo.toml', root / 'Cargo.lock']
paths.extend(path for path in (root / '.cargo').rglob('*') if path.is_file())
paths.extend(path for path in (root / 'crates').rglob('*') if path.is_file())
digest = hashlib.sha256()
for path in sorted(paths):
    if path.is_symlink():
        p.error('Build inputs must not be symbolic links')
    digest.update(path.relative_to(root).as_posix().encode() + b'\0')
    digest.update(hashlib.sha256(path.read_bytes()).digest())
result = dict(algorithm='sha256', source_sha256=digest.hexdigest(), files=len(paths))
if args.check and json.loads(args.check.read_text(encoding='utf-8')) != result:
    p.error('Application source changed during the build; rebuild from the final source tree')
if args.write:
    args.write.parent.mkdir(parents=True, exist_ok=True)
    args.write.write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8')
print(json.dumps(result))
