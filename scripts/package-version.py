"""Resolve the workspace version and reject packaging an already issued baseline."""

import re
import sys
from pathlib import Path


root = Path(__file__).resolve().parent.parent
manifest = (root / "Cargo.toml").read_text(encoding="utf-8")
match = re.search(
    r'(?ms)^\[workspace\.package\][ \t]*\n(?:(?!^\[).)*?^version\s*=\s*"(\d+\.\d+\.\d+)"',
    manifest,
)
if not match:
    sys.exit("Missing workspace package version")
version = match[1]
if "--for-package" in sys.argv[1:]:
    # Last version issued before the versioned output layout was introduced.
    if tuple(map(int, version.split("."))) <= (0, 1, 5):
        sys.exit("BitNet 0.1.5 was already packaged; increase the workspace version")
print(version)
