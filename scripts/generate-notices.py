#!/usr/bin/env python3
"""Collect original distribution notices from Cargo's offline target graph and cache.

This is a local evidence collector, not an SPDX expression interpreter or a
linker-level software bill of materials. It never fetches missing dependencies.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
from typing import Any


PROJECT = Path(__file__).resolve().parent.parent
MAX_DOCUMENT_BYTES = 8 * 1024 * 1024
MAX_DOCUMENTS_PER_PACKAGE = 512
MAX_SCAN_FILES = 50_000
LEGAL_NAME = re.compile(r"^(?:licen[cs]e|copying|copyright|notice|unlicense)(?:$|[-_. ])", re.I)
CODE_SUFFIXES = {".rs", ".c", ".h", ".cpp", ".py", ".js", ".toml", ".json", ".lock"}
SKIP_DIRECTORIES = {".git", "target", "node_modules", ".hg", ".svn"}
EMBEDDED_NOTICE_FILES = {"libsqlite3-sys": ("sqlite3/sqlite3.h",)}


def run_json(command: list[str], cwd: Path) -> dict[str, Any]:
    result = subprocess.run(command, cwd=cwd, capture_output=True, timeout=120, check=False)
    if result.returncode:
        raise RuntimeError(result.stderr.decode("utf-8", errors="replace").strip())
    return json.loads(result.stdout)


def active_cargo_packages(target: str) -> set[tuple[str, str]]:
    result = subprocess.run(
        ["cargo", "tree", "--offline", "--locked", "--workspace", "--target", target,
         "--edges", "normal,build", "--prefix", "none", "--format", "{p}", "--color", "never"],
        cwd=PROJECT, capture_output=True, timeout=120, check=False,
    )
    if result.returncode:
        raise RuntimeError(result.stderr.decode("utf-8", errors="replace").strip())
    packages = set()
    for line in result.stdout.decode("utf-8").splitlines():
        if not line:
            continue
        match = re.fullmatch(r"([A-Za-z0-9_-]+) v([A-Za-z0-9.+-]+)(?: .*)?", line)
        if match is None:
            raise ValueError("Unexpected package line in Cargo's target dependency tree")
        packages.add((match[1], match[2]))
    if not packages:
        raise ValueError("Cargo's target dependency tree is empty")
    return packages


def host_target() -> str:
    result = subprocess.run(["rustc", "-vV"], cwd=PROJECT, capture_output=True, timeout=30, check=True)
    for line in result.stdout.decode("utf-8").splitlines():
        if line.startswith("host: "):
            return line.removeprefix("host: ").strip()
    raise RuntimeError("rustc did not report its host target; pass --target explicitly")


def inside(path: Path, root: Path) -> bool:
    return path.is_relative_to(root)


def discover_documents(root: Path, declared: str | None) -> tuple[list[Path], list[str]]:
    """Find named legal documents, including bundled/native third-party notices."""
    found: set[Path] = set()
    warnings: list[str] = []
    if declared:
        path = Path(declared)
        path = path if path.is_absolute() else root / path
        found.add(path)
    scanned = 0
    for directory, directories, files in os.walk(root, followlinks=False):
        here = Path(directory)
        relative = here.relative_to(root)
        directories[:] = sorted(d for d in directories if d not in SKIP_DIRECTORIES)
        if len(relative.parts) >= 8:
            if directories:
                warnings.append(f"Not scanned below depth 8: {relative.as_posix()}")
            directories[:] = []
        legal_directory = any(part.lower() in {"license", "licenses", "licence", "licences", "legal"} for part in relative.parts)
        for name in sorted(files):
            scanned += 1
            if scanned > MAX_SCAN_FILES:
                warnings.append("Package scan reached the 50000-file limit")
                return sorted(found), warnings
            path = here / name
            if (LEGAL_NAME.match(name) and path.suffix.lower() not in CODE_SUFFIXES) or (
                legal_directory and path.suffix.lower() in {".txt", ".md", ".rst", ".html", ""}
            ):
                found.add(path)
                if len(found) >= MAX_DOCUMENTS_PER_PACKAGE:
                    warnings.append("Package scan reached the 512-document limit")
                    return sorted(found), warnings
    return sorted(found), warnings


def read_document(path: Path, root: Path, documents: dict[str, dict[str, Any]]) -> dict[str, Any]:
    resolved = path.resolve(strict=True)
    if not inside(resolved, root):
        raise ValueError("Document resolves outside the package directory")
    if not resolved.is_file():
        raise ValueError("Document is not a regular file")
    if resolved.stat().st_size > MAX_DOCUMENT_BYTES:
        raise ValueError("Document exceeds the 8 MiB limit")
    raw = resolved.read_bytes()
    if not raw:
        raise ValueError("Document is empty")
    encoding = "utf-8"
    try:
        # Preserve an original UTF-8 BOM if present, and every original character.
        text = raw.decode("utf-8")
    except UnicodeDecodeError:
        encoding = "latin-1"
        text = raw.decode(encoding)
    if "\x00" in text:
        raise ValueError("Document contains NUL bytes and is not accepted as text")
    digest = hashlib.sha256(raw).hexdigest()
    documents.setdefault(digest, {"sha256": digest, "bytes": len(raw), "encoding": encoding, "text": text})
    return {"path": path.relative_to(root).as_posix(), "sha256": digest}


def embedded_source_notices(name: str, root: Path, documents: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
    result = []
    for relative in EMBEDDED_NOTICE_FILES.get(name, ()):
        path = (root / relative).resolve(strict=True)
        if not inside(path, root):
            raise ValueError("Embedded-notice source resolves outside its package")
        with path.open("rb") as stream:
            prefix = stream.read(64 * 1024)
        end = prefix.find(b"*/")
        if not prefix.startswith(b"/*") or end < 0:
            raise ValueError("Known bundled-library source has no bounded leading comment")
        original = prefix[:end + 2]
        text = original.decode("utf-8")
        digest = hashlib.sha256(original).hexdigest()
        source_digest = hashlib.sha256()
        with path.open("rb") as stream:
            for block in iter(lambda: stream.read(1024 * 1024), b""):
                source_digest.update(block)
        documents.setdefault(digest, {"sha256": digest, "bytes": len(original), "encoding": "utf-8", "text": text})
        result.append({
            "path": relative + ":leading-comment", "sha256": digest,
            "provenance": {"kind": "embedded-source-comment", "source_path": relative, "source_sha256": source_digest.hexdigest(), "byte_start": 0, "byte_end_exclusive": end + 2, "line_start": 1, "line_end": original.count(b"\n") + 1},
        })
    return result


def selected_packages(metadata: dict[str, Any]) -> set[str]:
    """Conservative normal + build closure of all workspace products, excluding dev-only edges."""
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    pending = list(metadata["workspace_members"])
    selected: set[str] = set()
    while pending:
        package_id = pending.pop()
        if package_id in selected:
            continue
        selected.add(package_id)
        for dependency in nodes[package_id]["deps"]:
            if any(kind.get("kind") != "dev" for kind in dependency["dep_kinds"]):
                pending.append(dependency["pkg"])
    return selected


def vcs_info(package_root: Path, source: str) -> dict[str, str] | None:
    if source.startswith("git+"):
        commit = source.rpartition("#")[2]
        if not re.fullmatch(r"[a-f0-9]{40}", commit):
            return None
        result = subprocess.run(
            ["git", "rev-parse", "--show-toplevel", "HEAD"], cwd=package_root,
            capture_output=True, timeout=30, check=False,
        )
        checkout = result.stdout.decode("utf-8").splitlines()
        if result.returncode or len(checkout) != 2 or checkout[1] != commit:
            return None
        root = Path(checkout[0]).resolve()
        if not inside(package_root, root):
            return None
        relative = package_root.relative_to(root)
        return {"commit": commit, "path_in_vcs": relative.as_posix() if relative.parts else ""}
    path = package_root / ".cargo_vcs_info.json"
    if not path.is_file():
        return None
    data = json.loads(path.read_text(encoding="utf-8"))
    commit = data.get("git", {}).get("sha1", "")
    if not re.fullmatch(r"[a-f0-9]{40}", commit):
        return None
    return {"commit": commit, "path_in_vcs": data.get("path_in_vcs", "")}


def complete_root_licenses(entries: list[dict[str, Any]], documents: dict[str, dict[str, Any]]) -> None:
    """Use root licenses only with exact repository, commit and SPDX matches."""
    candidates = {}
    for entry in entries:
        vcs = entry.get("vcs")
        if not vcs or vcs["path_in_vcs"] or not entry["repository"]:
            continue
        files = [item for item in entry["license_files"] if len(Path(item["path"]).parts) == 1 and item["path"].lower().startswith(("license", "licence", "copying", "unlicense"))]
        if files:
            candidates[(entry["repository"], vcs["commit"], entry["spdx"])] = (entry, files)
    for entry in entries:
        vcs = entry.get("vcs")
        if entry["license_files"] or not vcs:
            continue
        matching = candidates.get((entry["repository"], vcs["commit"], entry["spdx"]))
        if matching:
            source, files = matching
            for item in files:
                entry["license_files"].append({
                    "path": "repository-root:" + item["path"], "sha256": item["sha256"],
                    "provenance": {"kind": "same-repository-commit-root-license", "from_package": f"{source['name']} {source['version']}", "repository": entry["repository"], "commit": vcs["commit"]},
                })
    manifest = PROJECT / "docs/licenses/upstream-provenance.json"
    if not manifest.is_file():
        return
    supplemental = json.loads(manifest.read_text(encoding="utf-8"))
    for entry in entries:
        if entry["license_files"] or not entry.get("vcs"):
            continue
        for supplied in supplemental:
            if not all(supplied.get(key) == entry[key] for key in ("name", "version", "repository")) or supplied.get("commit") != entry["vcs"]["commit"]:
                continue
            path = manifest.parent / supplied["file"]
            item = read_document(path, manifest.parent.resolve(), documents)
            if item["sha256"] != supplied["sha256"]:
                raise ValueError(f"Supplemental license SHA256 mismatch for {entry['name']}")
            expected_prefix = entry["repository"].replace("https://github.com/", "https://raw.githubusercontent.com/").removesuffix(".git") + "/" + entry["vcs"]["commit"] + "/"
            if not supplied["url"].startswith(expected_prefix):
                raise ValueError(f"Supplemental license URL does not identify the exact source commit for {entry['name']}")
            item["path"] = "recorded-upstream:" + item["path"]
            item["provenance"] = {"kind": "recorded-exact-upstream-commit", "repository": entry["repository"], "commit": entry["vcs"]["commit"], "url": supplied["url"]}
            entry["license_files"].append(item)


def bundled_font_notices(root: Path, documents: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
    folder = root / "crates/bitnet-gui/assets/fonts"
    manifest = folder / "manifest.json"
    if not manifest.is_file():
        return []
    record = json.loads(manifest.read_text(encoding="utf-8"))
    license_document = read_document(folder / record["license_file"], folder, documents)
    if license_document["sha256"] != record["license_sha256"]:
        raise ValueError("Bundled font license SHA256 mismatch")
    assets = []
    for supplied in record["files"]:
        path = folder / supplied["file"]
        resolved = path.resolve(strict=True)
        if not inside(resolved, folder.resolve()) or not resolved.is_file():
            raise ValueError("Bundled font resolves outside its asset directory")
        digest = hashlib.sha256(resolved.read_bytes()).hexdigest()
        if digest != supplied["sha256"] or resolved.stat().st_size != supplied["bytes"]:
            raise ValueError(f"Bundled font SHA256/size mismatch for {supplied['file']}")
        assets.append(dict(supplied))
    if len(assets) != 2:
        raise ValueError("Expected the regular and medium bundled fonts")
    license_document["provenance"] = {
        "kind": "bundled-subset-font", "repository": record["repository"],
        "commit": record["commit"], "url": record["license_url"],
    }
    return [{
        "name": record["name"], "version": record["version"], "spdx": record["spdx"],
        "source": record["repository"] + "#" + record["commit"],
        "license_files": [license_document], "assets": assets,
    }]


def collect(
    metadata: dict[str, Any], target: str, extras: list[str], active: set[tuple[str, str]],
) -> dict[str, Any]:
    root = Path(metadata["workspace_root"]).resolve()
    members = set(metadata["workspace_members"])
    selected = selected_packages(metadata)
    # Metadata can include optional dependencies reached through weak feature
    # references. Cargo's target tree shows which packages are actually active.
    packages = [
        p for p in metadata["packages"]
        if p["id"] in selected and (p["name"], p["version"]) in active
    ]
    missing = active - {(p["name"], p["version"]) for p in packages}
    if missing:
        names = ", ".join(f"{name} {version}" for name, version in sorted(missing))
        raise ValueError(f"Active Cargo dependencies missing from metadata: {names}")
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    documents: dict[str, dict[str, Any]] = {}
    entries: list[dict[str, Any]] = []
    issues: list[dict[str, str]] = []
    project_documents = []
    for relative in ("LICENSE", "crates/bitnet-gateway/NOTICE"):
        project_documents.append(read_document(root / relative, root, documents))
    for package in sorted(packages, key=lambda p: (p["name"], p["version"], p["id"])):
        package_root = Path(package["manifest_path"]).resolve().parent
        workspace = package["id"] in members
        source = package["source"]
        if source is None:
            source = "workspace:" + package_root.relative_to(root).as_posix() if inside(package_root, root) else "local-path-dependency"
        entry: dict[str, Any] = {
            "name": package["name"], "version": package["version"], "source": source,
            "spdx": package.get("license"), "workspace": workspace,
            "repository": package.get("repository"), "vcs": vcs_info(package_root, source),
            "features": sorted(nodes[package["id"]]["features"]),
            "license_files": [], "warnings": [],
        }
        paths, warnings = discover_documents(package_root, package.get("license_file"))
        entry["warnings"].extend(warnings)
        for path in paths:
            relative = path.relative_to(package_root).as_posix() if inside(path, package_root) else "declared-license-file"
            try:
                entry["license_files"].append(read_document(path, package_root, documents))
            except (OSError, ValueError) as error:
                entry["warnings"].append(f"{relative}: {error}")
        try:
            entry["license_files"].extend(embedded_source_notices(entry["name"], package_root, documents))
        except (OSError, ValueError) as error:
            entry["warnings"].append(f"Known bundled-library notice extraction failed: {error}")
        # These packages are this workspace's own products. Preserve the actual
        # project license as an explicit inherited document, not an invented text.
        if workspace:
            entry["license_files"].append({"path": "workspace:LICENSE", "sha256": project_documents[0]["sha256"]})
        entries.append(entry)
    complete_root_licenses(entries, documents)
    for entry in entries:
        if not entry["spdx"]:
            entry["warnings"].append("Cargo metadata has no SPDX license expression")
        if not entry["license_files"]:
            entry["warnings"].append("No original LICENSE/COPYING/NOTICE document found in the local package")
        for warning in entry["warnings"]:
            issues.append({"package": f"{entry['name']} {entry['version']}", "detail": warning})
    external_documents = bundled_font_notices(root, documents)
    for value in extras:
        # LABEL=PATH is optional. A Windows drive colon is never split.
        label, supplied = value.split("=", 1) if "=" in value else (Path(value).name, value)
        path = Path(supplied).resolve(strict=True)
        document = read_document(path, path.parent, documents)
        external_documents.append({"name": label, "source": "explicit-local-file", "version": None, "spdx": None, "license_files": [document]})
    return {
        "format_version": 1, "target": target,
        "scope": "Offline Cargo normal/build dependency closure of all workspace members for the selected target. Dev-only edges excluded; build dependencies included conservatively. Not an exact linker SBOM. Features are Cargo-resolved metadata, which may be unified with dev features.",
        "project_documents": project_documents,
        "packages": entries, "external_documents": external_documents,
        "documents": [documents[key] for key in sorted(documents)],
        "issues": issues,
        "summary": {
            "packages": len(entries), "external_documents": len(external_documents),
            "unique_documents": len(documents), "issues": len(issues),
            "packages_without_original_text": sum(not entry["license_files"] for entry in entries),
        },
    }


def render(report: dict[str, Any]) -> str:
    lines = [
        "BitNet - original project and third-party license notices", "=" * 72,
        f"Target: {report['target']}", report["scope"],
        "Original texts below are read from local files. SPDX expressions are copied",
        "from Cargo metadata without choosing among alternative license terms.",
        "Repeated identical documents are included once and referenced by SHA256.",
        "Missing documents and collection limits are explicitly listed below.", "",
        "PROJECT DOCUMENTS", "-" * 72,
    ]
    for item in report["project_documents"]:
        lines.append(f"{item['path']} -> SHA256 {item['sha256']}")
    lines += ["", "PACKAGE INDEX", "-" * 72]
    for package in report["packages"] + report["external_documents"]:
        lines += ["", f"{package['name']} {package.get('version') or '(version not supplied)'}",
                  f"Source: {package['source']}", f"SPDX: {package.get('spdx') or '(not supplied; see original text)'}"]
        for item in package["license_files"]:
            lines.append(f"  {item['path']} -> SHA256 {item['sha256']}")
            if item.get("provenance"):
                lines.append("    Provenance: " + json.dumps(item["provenance"], ensure_ascii=False, sort_keys=True))
        for asset in package.get("assets", []):
            lines.append(f"  Bundled font: {asset['file']} -> SHA256 {asset['sha256']}")
            lines.append(f"    Upstream: {asset['source_url']} -> SHA256 {asset['source_sha256']}")
        for warning in package.get("warnings", []):
            lines.append(f"  COLLECTION WARNING: {warning}")
    lines += ["", "COLLECTION ISSUES", "-" * 72]
    lines += [f"{item['package']}: {item['detail']}" for item in report["issues"]] or ["None detected within the collection scope."]
    lines += ["", "ORIGINAL DOCUMENT TEXTS (deduplicated by original-byte SHA256)", "=" * 72]
    output = "\n".join(lines) + "\n"
    for document in report["documents"]:
        output += f"\nDOCUMENT SHA256 {document['sha256']}\nOriginal encoding: {document['encoding']}; bytes: {document['bytes']}\n{'-' * 72}\n"
        output += document["text"]
        if not document["text"].endswith("\n"):
            output += "\n"
        output += f"\nEND DOCUMENT SHA256 {document['sha256']}\n"
    return output


def atomic_write(path: Path, text: str) -> None:
    temporary = path.with_suffix(path.suffix + f".{os.getpid()}.tmp")
    try:
        with temporary.open("w", encoding="utf-8", newline="") as stream:
            stream.write(text)
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", help="Rust target triple (default: rustc host)")
    parser.add_argument("--output-dir", type=Path, default=PROJECT / "build/licenses")
    parser.add_argument("--extra-license", action="append", default=[], metavar="[LABEL=]PATH", help="Additional local native-library/license document; repeatable")
    parser.add_argument("--strict", action="store_true", help="Write the report, then fail if any collection issue is found")
    arguments = parser.parse_args()
    try:
        host = host_target()
        target = arguments.target or host
        if not re.fullmatch(r"[A-Za-z0-9_.-]+", target):
            raise ValueError("Invalid target triple")
        output = arguments.output_dir.resolve()
        if not inside(output, PROJECT):
            raise ValueError("Output directory must be inside this project")
        command = ["cargo", "metadata", "--offline", "--locked", "--format-version", "1", "--filter-platform", target]
        if target != host:
            command += ["--filter-platform", host]
        metadata = run_json(command, PROJECT)
        report = collect(metadata, target, arguments.extra_license, active_cargo_packages(target))
        output.mkdir(parents=True, exist_ok=True)
        atomic_write(output / "THIRD-PARTY-NOTICES.txt", render(report))
        atomic_write(output / "THIRD-PARTY-NOTICES.json", json.dumps(report, ensure_ascii=False, indent=2) + "\n")
        print(json.dumps({"target": target, "output": output.relative_to(PROJECT).as_posix(), **report["summary"]}, ensure_ascii=False))
        for issue in report["issues"]:
            print(f"Collection issue: {issue['package']}: {issue['detail']}", file=sys.stderr)
        return 2 if arguments.strict and report["issues"] else 0
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"Notice collection failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
