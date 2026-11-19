#!/usr/bin/env python3
"""Normalize portable archive ownership/modes independently of the staging filesystem."""
import gzip
import os
from pathlib import Path
import sys
import tarfile

source, output = map(Path, sys.argv[1:])
epoch = int(os.environ.get('SOURCE_DATE_EPOCH', '0'))
with output.open('wb') as stream, gzip.GzipFile(filename='', mode='wb', fileobj=stream, mtime=epoch) as compressed:
    with tarfile.open(fileobj=compressed, mode='w') as tar:
        for path in [source, *sorted(source.rglob('*'))]:
            if path.is_symlink() or not (path.is_file() or path.is_dir()):
                raise ValueError('Unexpected portable payload type')
            info = tarfile.TarInfo(path.relative_to(source.parent).as_posix())
            info.uid = info.gid = 0
            info.uname = info.gname = 'root'
            info.mtime = epoch
            executable = path.parent == source and path.name in {
                "bitengine", "bitctl", "bitmaint", "bitnet-gui", "bitnet-tui", "bitnet-web",
            }
            info.mode = 0o755 if path.is_dir() or executable else 0o644
            if path.is_dir():
                info.type = tarfile.DIRTYPE
                tar.addfile(info)
            else:
                info.size = path.stat().st_size
                with path.open('rb') as payload:
                    tar.addfile(info, payload)
