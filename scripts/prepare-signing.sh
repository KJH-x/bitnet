#!/bin/sh
# Prepare an ignored development key, or validate externally supplied release keys.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
minisign=${MINISIGN:-minisign}
if [ "${1:-}" = --development ]; then
    keys=$root/.local/release-keys
    mkdir -p "$keys"; chmod 0700 "$keys"
    secret=$keys/development.key; public=$keys/development.pub
    if [ ! -e "$secret" ]; then
        umask 077
        "$minisign" -G -W -s "$secret" -p "$public" >/dev/null 2>&1
    fi
    chmod 0600 "$secret"
else
    secret=${BITNET_RELEASE_SECRET_KEY_FILE:?External private-key file is required}
    public=${BITNET_RELEASE_PUBLIC_KEY_FILE:?External public-key file is required}
fi
test -f "$secret"; test -f "$public"
key=$(sed '/^untrusted comment:/d; /^[[:space:]]*$/d' "$public" | tr -d '\r')
case "$key" in ''|*[!A-Za-z0-9+/=]*) echo 'Invalid Minisign public-key text' >&2; exit 2;; esac
mkdir -p "$root/.local/release-keys"
printf 'BITNET_RELEASE_PUBLIC_KEY=%s\n' "$key" > "$root/.local/release-keys/public.env"
printf 'Prepared signing inputs; export the public.env value before compiling.\n'
