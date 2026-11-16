#!/bin/sh
set -eu
umask 077

# The engine and user login entries can outlive the AppImage mount.
image=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
payload=$image/usr/lib/bitnet
key=$(sha256sum "$payload/SHA256SUMS")
key=${key%% *}
base=${XDG_DATA_HOME:-$HOME/.local/share}/bitnet/portable
case "$base" in /*) ;; *) echo 'BitNet user data path must be absolute.' >&2; exit 1;; esac
mkdir -p -- "$base"
if [ -L "$base" ] || [ "$(stat -c %u -- "$base")" != "$(id -u)" ]; then
    echo 'BitNet portable directory must belong to this user.' >&2
    exit 1
fi
chmod 0700 -- "$base"

dest=$base/$key
if [ ! -e "$dest" ] && [ ! -L "$dest" ]; then
    stage=$(mktemp -d "$base/.stage.XXXXXXXX")
    # Concurrent launchers publish once. The losing stage is never executed.
    cleanup() {
        for program in bitengine bitctl bitmaint bitnet-gui SHA256SUMS; do
            rm -f -- "$stage/$program"
        done
        rmdir -- "$stage"
    }
    trap cleanup EXIT
    trap 'exit 1' HUP INT TERM
    for program in bitengine bitctl bitmaint bitnet-gui; do
        install -m 0700 -- "$payload/$program" "$stage/$program"
    done
    install -m 0600 -- "$payload/SHA256SUMS" "$stage/SHA256SUMS"
    (cd -- "$stage" && sha256sum --quiet -c SHA256SUMS)
    mv -nT -- "$stage" "$dest"
    if [ -d "$stage" ]; then cleanup; fi
    trap - EXIT HUP INT TERM
fi

if [ -L "$dest" ] || [ "$(stat -c '%u:%a' -- "$dest")" != "$(id -u):700" ]; then
    echo 'BitNet portable directory is not private.' >&2
    exit 1
fi
for program in bitengine bitctl bitmaint bitnet-gui SHA256SUMS; do
    if [ -L "$dest/$program" ] || [ ! -f "$dest/$program" ]; then
        echo 'Invalid BitNet portable payload. Remove its version directory and try again.' >&2
        exit 1
    fi
done
cmp -s -- "$payload/SHA256SUMS" "$dest/SHA256SUMS"
(cd -- "$dest" && sha256sum --quiet -c SHA256SUMS)
case "${1:-}" in
    --cli) shift; exec "$dest/bitctl" "$@" ;;
    *) exec "$dest/bitnet-gui" "$@" ;;
esac
