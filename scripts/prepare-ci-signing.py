"""Provision a runner-local production signing key and pass its path to jobs."""
import os
from pathlib import Path
import re
import tempfile


def main() -> None:
    secret = os.environ["BITNET_SIGNING_SECRET"]
    public = os.environ["BITNET_SIGNING_PUBLIC"].strip()
    lines = [line.strip() for line in public.splitlines() if line.strip() and not line.startswith("untrusted comment:")]
    if not secret.strip() or len(lines) != 1 or not re.fullmatch(r"[A-Za-z0-9+/=]{56}", lines[0]):
        raise SystemExit("Invalid Minisign key inputs")
    if len(public.splitlines()) == 1:
        public = "untrusted comment: BitNet release public key\n" + public
    folder = Path(tempfile.mkdtemp(prefix="bitnet-signing-", dir=os.environ["RUNNER_TEMP"]))
    secret_path = folder / "release.key"
    public_path = folder / "release.pub"
    with os.fdopen(os.open(secret_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w", encoding="ascii") as f:
        f.write(secret.rstrip() + "\n")
    public_path.write_text(public + "\n", encoding="ascii")
    with Path(os.environ["GITHUB_ENV"]).open("a", encoding="utf-8") as f:
        f.write(f"BITNET_RELEASE_SECRET_KEY_FILE={secret_path}\n")
        f.write(f"BITNET_RELEASE_PUBLIC_KEY_FILE={public_path}\n")
        f.write(f"BITNET_RELEASE_PUBLIC_KEY={lines[0]}\n")


if __name__ == "__main__":
    main()
