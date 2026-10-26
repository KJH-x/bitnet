#!/usr/bin/env python3
"""Fingerprint application build inputs; fail if they changed during a release build."""
import argparse
import hashlib
import json
from pathlib import Path

GENERATED_DIRECTORIES = {'.local', '.git', 'target', '__pycache__'}


def fingerprint(root: Path) -> dict:
    paths = [root / 'Cargo.toml', root / 'Cargo.lock']
    for directory in ('.cargo', 'crates'):
        for path in (root / directory).rglob('*'):
            if GENERATED_DIRECTORIES.intersection(path.relative_to(root).parts):
                continue
            if path.is_symlink():
                raise ValueError('Build inputs must not be symbolic links')
            if path.is_file():
                paths.append(path)
    digest = hashlib.sha256()
    # Use the same path ordering on Windows and Unix.
    for path in sorted(paths, key=lambda item: item.relative_to(root).as_posix()):
        if path.is_symlink():
            raise ValueError('Build inputs must not be symbolic links')
        digest.update(path.relative_to(root).as_posix().encode() + b'\0')
        digest.update(hashlib.sha256(path.read_bytes()).digest())
    return dict(algorithm='sha256', source_sha256=digest.hexdigest(), files=len(paths))


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument('--write', type=Path)
    p.add_argument('--check', type=Path)
    args = p.parse_args()
    try:
        result = fingerprint(Path(__file__).resolve().parent.parent)
    except (ValueError, OSError) as error:
        p.error(str(error))
    if args.check and json.loads(args.check.read_text(encoding='utf-8')) != result:
        p.error('Application source changed during the build; rebuild from the final source tree')
    if args.write:
        args.write.parent.mkdir(parents=True, exist_ok=True)
        args.write.write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
