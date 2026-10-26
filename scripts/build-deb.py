#!/usr/bin/env python3
"""Build Debian ar/tar containers with explicit root ownership and modes, also on WSL/NTFS."""
import gzip
import os
from pathlib import Path
import shutil
import sys
import tarfile

root, output = map(Path, sys.argv[1:])
epoch = int(os.environ.get('SOURCE_DATE_EPOCH', '0'))
members = []
for control in (True, False):
    base = root / 'DEBIAN' if control else root
    archive = root.parent / ('control.tar.gz' if control else 'data.tar.gz')
    with archive.open('wb') as stream, gzip.GzipFile(filename='', mode='wb', fileobj=stream, mtime=epoch) as compressed:
        with tarfile.open(fileobj=compressed, mode='w') as tar:
            for path in sorted(base.rglob('*')):
                relative = path.relative_to(base)
                if not control and relative.parts[0] == 'DEBIAN':
                    continue
                if path.is_symlink() or not (path.is_file() or path.is_dir()):
                    raise ValueError('Only regular files/directories may enter a Debian payload')
                info = tarfile.TarInfo('./' + relative.as_posix())
                info.uid = info.gid = 0
                info.uname = info.gname = 'root'
                info.mtime = epoch
                executable = (control and relative.name in {'postinst','prerm','postrm'}) or relative.parts[:2] == ('usr','bin') or relative.parts[:3] == ('usr','libexec','bitnet')
                info.mode = 0o755 if path.is_dir() or executable else 0o644
                if path.is_dir():
                    info.type = tarfile.DIRTYPE
                    tar.addfile(info)
                else:
                    info.size = path.stat().st_size
                    with path.open('rb') as payload:
                        tar.addfile(info, payload)
    members.append(archive)
version = root.parent / 'debian-binary'
version.write_bytes(b'2.0\n')
output.parent.mkdir(parents=True, exist_ok=True)
with output.open('wb') as package:
    package.write(b'!<arch>\n')
    for member in [version, *members]:
        size = member.stat().st_size
        header = f'{member.name + "/":<16}{epoch:<12}{0:<6}{0:<6}{"100644":<8}{size:<10}`\n'
        if len(header) != 60:
            raise ValueError('Invalid Debian ar member header')
        package.write(header.encode('ascii'))
        with member.open('rb') as payload:
            shutil.copyfileobj(payload, package)
        if size % 2:
            package.write(b'\n')
print(f'Built {output.name} with explicit root ownership and executable/data modes.')
