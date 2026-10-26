#!/usr/bin/env python3
"""Stage release assets and a schema-v1 manifest. Does not upload or sign anything."""
import argparse
import hashlib
import json
import re
from pathlib import Path
import shutil
from urllib.parse import quote, urlparse

p = argparse.ArgumentParser()
p.add_argument('--spec', required=True, type=Path, help='JSON array: path/platform/arch/kind/filename')
p.add_argument('--version', required=True)
p.add_argument('--base-url', required=True, action='append')
p.add_argument('--out', required=True, type=Path)
args = p.parse_args()
if not re.fullmatch(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', args.version):
    p.error('Use a stable semantic version without a prefix')
if len(args.base_url) > 3:
    p.error('The update protocol permits at most three download mirrors')
for base in args.base_url:
    u = urlparse(base)
    if u.scheme != 'https' or not u.netloc or u.username or u.password or u.query or u.fragment:
        p.error('Every base URL must be an HTTPS directory URL without credentials/query/fragment')
args.out.mkdir(parents=True, exist_ok=True)
assets = []
names = set()
identities = set()
formats = {'windows': {'client-msi': '.msi', 'service-msi': '.msi'},
           'linux': {'core-deb': '.deb', 'gui-deb': '.deb'},
           'macos': {'client-pkg': '.pkg', 'service-pkg': '.pkg'}}
for entry in json.loads(args.spec.read_text(encoding='utf-8-sig')):
    source = Path(entry['path']).resolve(strict=True)
    name = entry.get('filename', source.name)
    identity = (entry['platform'], entry['arch'], entry['kind'])
    suffix = formats.get(entry['platform'], {}).get(entry['kind'])
    reserved = name.split('.')[0].upper() in {'CON','PRN','AUX','NUL',*[f'COM{i}' for i in range(1,10)],*[f'LPT{i}' for i in range(1,10)]}
    if not suffix or entry['arch'] not in {'x86', 'x86_64', 'aarch64'} or (entry['platform'] == 'macos' and entry['arch'] == 'x86'):
        p.error('Unsupported update platform/architecture/kind')
    if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.-]*', name) or '..' in name or reserved or not name.lower().endswith(suffix):
        p.error('Unsafe update asset filename or mismatched extension')
    if name != Path(name).name or name in names or identity in identities or not source.is_file():
        p.error('Asset names must be unique basenames and assets must be files')
    names.add(name)
    identities.add(identity)
    destination = args.out / name
    if source != destination.resolve():
        shutil.copyfile(source, destination)
    with destination.open('rb') as data:
        hasher = hashlib.sha256()
        for block in iter(lambda: data.read(1024 * 1024), b''):
            hasher.update(block)
        digest = hasher.hexdigest()
    assets.append(dict(platform=entry['platform'], arch=entry['arch'], kind=entry['kind'], filename=name,
                       bytes=destination.stat().st_size, sha256=digest,
                       urls=[base.rstrip('/') + '/' + quote(name) for base in args.base_url]))
    signature = source.with_name(source.name + '.minisig')
    if signature.is_file():
        shutil.copyfile(signature, args.out / (name + '.minisig'))
manifest = dict(schema_version=1, api_version=1, version=args.version, assets=assets)
(args.out / 'manifest.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
(args.out / 'SHA256SUMS').write_text(''.join(f"{a['sha256']}  {a['filename']}\n" for a in assets), encoding='ascii')
print(f'Staged {len(assets)} assets; manifest.json still requires release signing.')
