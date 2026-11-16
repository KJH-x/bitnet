#!/bin/sh
# Pinned upstream binaries, stored in the project only.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
case "$(uname -m)" in
    x86_64)
        arch=x86_64
        runtime_sha=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d
        ;;
    aarch64)
        arch=aarch64
        runtime_sha=00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444
        ;;
    *) echo 'Unsupported host architecture for AppImage builds' >&2; exit 2;;
esac
dest=$root/.local/tools/appimage
mkdir -p "$dest"
fetch() {
    url=$1 file=$2 digest=$3
    if [ ! -f "$file" ]; then
        download=$(mktemp "$dest/.download.XXXXXXXX")
        trap 'rm -f -- "$download"' EXIT
        trap 'exit 1' HUP INT TERM
        curl --proto '=https' --proto-redir '=https' --retry 2 --max-time 120 -fsSL "$url" -o "$download"
        printf '%s  %s\n' "$digest" "$download" | sha256sum --check --status
        mv -- "$download" "$file"
        trap - EXIT HUP INT TERM
    fi
    printf '%s  %s\n' "$digest" "$file" | sha256sum --check --status
}
runtime=$dest/runtime-20251108-$arch
fetch "https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-$arch" "$runtime" "$runtime_sha"
printf 'APPIMAGE_RUNTIME=%s\n' "$runtime"
