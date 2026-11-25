#!/bin/sh
# Build-only: never executes maintainer scripts, starts services, or installs packages.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
source_version=$(python3 "$root/scripts/package-version.py" --for-package)
version=${VERSION:-$source_version}
arch=${ARCH:-$(dpkg --print-architecture)}
glibc_min=${GLIBC_MIN:-$(getconf GNU_LIBC_VERSION | awk '{print $2}')}
case "$arch" in amd64|arm64|i386) ;; *) echo 'Supported package architectures: amd64 arm64 i386' >&2; exit 2;; esac
bin=${BIN_DIR:-$root/target/release}
python3 "$root/scripts/fingerprint-source.py" --check "$bin/bitnet-build-source.json" >/dev/null
python3 - "$root/Cargo.toml" "$version" <<'PY'
import re, sys
from pathlib import Path
match = re.search(r'(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"', Path(sys.argv[1]).read_text(encoding='utf-8'))
if not match or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", sys.argv[2]) or match[1] != sys.argv[2]:
    raise SystemExit('Package version must match workspace.package.version')
PY
minisign=${MINISIGN:-minisign}
if [ "${DEVELOPMENT:-0}" = 1 ]; then
    secret=$root/.local/release-keys/development.key; public=$root/.local/release-keys/development.pub
else
    secret=${BITNET_RELEASE_SECRET_KEY_FILE:?External signing key required, or explicit DEVELOPMENT=1}
    public=${BITNET_RELEASE_PUBLIC_KEY_FILE:?External public key required}
fi
test -n "${BITNET_RELEASE_PUBLIC_KEY:-}"; test -f "$secret"; test -f "$public"
# With no update feed, LLVM can remove the unreachable update verifier/key entirely.
if [ -n "${BITNET_UPDATE_FEED:-}" ]; then
    grep -aF "$BITNET_RELEASE_PUBLIC_KEY" "$bin/bitmaint" >/dev/null || { echo 'Rebuild bitmaint with this public key' >&2; exit 2; }
fi
out=$root/dist/BitNet-$version-linux-$arch
test ! -f "$out/SHA256SUMS" || { echo "Packages already exist for $version; increment the version before packaging again" >&2; exit 2; }
target=$(case "$arch" in amd64) printf x86_64-unknown-linux-gnu;; arm64) printf aarch64-unknown-linux-gnu;; i386) printf i686-unknown-linux-gnu;; esac)
licenses=$root/build/licenses/linux-$arch
mkdir -p "$licenses/native"
set --
for copyright in "$root/.local/linux-openssl/OpenSSL-COPYRIGHT.txt" /usr/share/doc/libssl3/copyright /usr/share/doc/libssl3t64/copyright; do
    if [ -r "$copyright" ]; then cp -L "$copyright" "$licenses/native/OpenSSL-COPYRIGHT.txt"; set -- "$@" --extra-license "OpenSSL=$licenses/native/OpenSSL-COPYRIGHT.txt"; break; fi
done
if [ -r /usr/share/common-licenses/Apache-2.0 ]; then
    cp /usr/share/common-licenses/Apache-2.0 "$licenses/native/OpenSSL-APACHE-2.0.txt"
    set -- "$@" --extra-license "OpenSSL-Apache-2.0=$licenses/native/OpenSSL-APACHE-2.0.txt"
fi
python3 "$root/scripts/generate-notices.py" --target "$target" --output-dir "$licenses" --strict "$@"
mkdir -p "$root/build"
stage=${BUILD_DIR:-$(mktemp -d "$root/build/linux-$arch.XXXXXXXX")}
test -d "$stage" || mkdir -p "$stage"
mkdir -p "$stage/core/DEBIAN" "$stage/core/usr/bin" "$stage/core/usr/libexec/bitnet" "$stage/core/usr/lib/bitnet/templates" "$stage/core/lib/systemd/system" "$stage/gui/DEBIAN" "$stage/gui/usr/bin" "$stage/gui/usr/share/applications" "$stage/portable/bitnet-$version"
mkdir -p "$stage/gui/usr/share/icons/hicolor/scalable/apps"
for name in bitengine bitctl bitmaint; do install -m 0755 "$bin/$name" "$stage/core/usr/bin/$name"; done
install -m 0755 "$bin/bitmaint" "$stage/core/usr/libexec/bitnet/bitmaint"
for name in bitnet-gui bitnet-tui bitnet-web; do install -m 0755 "$bin/$name" "$stage/gui/usr/bin/$name"; done
install -m 0644 "$root/installers/linux/bitnet.service" "$stage/core/lib/systemd/system/bitnet.service"
install -m 0644 "$root/installers/linux/bitnet.service" "$stage/core/usr/lib/bitnet/templates/bitnet.service"
for script in postinst prerm postrm; do install -m 0755 "$root/installers/linux/$script" "$stage/core/DEBIAN/$script"; done
install -m 0644 "$root/installers/linux/bitnet.desktop" "$stage/gui/usr/share/applications/bitnet.desktop"
install -m 0644 "$root/installers/linux/bitnet.svg" "$stage/gui/usr/share/icons/hicolor/scalable/apps/bitnet.svg"
for package in core gui; do
    docs=$stage/$package/usr/share/doc/bitnet-$package
    mkdir -p "$docs"
    install -m 0644 "$root/LICENSE" "$docs/LICENSE"
    install -m 0644 "$root/crates/bitnet-gateway/NOTICE" "$docs/GATEWAY-NOTICE.txt"
    install -m 0644 "$licenses/THIRD-PARTY-NOTICES.txt" "$docs/THIRD-PARTY-NOTICES.txt"
    install -m 0644 "$licenses/THIRD-PARTY-NOTICES.json" "$docs/THIRD-PARTY-NOTICES.json"
done
install -m 0644 "$root/crates/bitnet-gui/assets/fonts/LICENSE.txt" "$stage/gui/usr/share/doc/bitnet-gui/FONT-OFL.txt"
cat > "$stage/core/DEBIAN/control" <<EOF
Package: bitnet-core
Version: $version
Architecture: $arch
Maintainer: BitNet contributors <maintainers@example.invalid>
Depends: libc6 (>= $glibc_min), libssl3, systemd (>= 249)
Suggests: libsecret-tools, gnome-keyring | keepassxc
Section: net
Priority: optional
Description: BitNet unattended campus network engine and CLI
EOF
cat > "$stage/gui/DEBIAN/control" <<EOF
Package: bitnet-gui
Version: $version
Architecture: $arch
Maintainer: BitNet contributors <maintainers@example.invalid>
Depends: bitnet-core (= $version), libx11-6, libxkbcommon0, libwayland-client0, libfontconfig1
Recommends: libsecret-tools, gnome-keyring | keepassxc
Section: net
Priority: optional
Description: BitNet graphical and terminal clients with an on-demand log viewer
EOF
mkdir -p "$out"
python3 "$root/scripts/build-deb.py" "$stage/core" "$out/bitnet-core_${version}_${arch}.deb"
python3 "$root/scripts/build-deb.py" "$stage/gui" "$out/bitnet-gui_${version}_${arch}.deb"
dpkg-deb --info "$out/bitnet-core_${version}_${arch}.deb" >/dev/null
dpkg-deb --info "$out/bitnet-gui_${version}_${arch}.deb" >/dev/null
for name in bitengine bitctl bitmaint bitnet-gui bitnet-tui bitnet-web; do install -m 0755 "$bin/$name" "$stage/portable/bitnet-$version/$name"; done
cp "$root/LICENSE" "$stage/portable/bitnet-$version/LICENSE"
mkdir -p "$stage/portable/bitnet-$version/licenses"
cp "$root/crates/bitnet-gateway/NOTICE" "$stage/portable/bitnet-$version/licenses/GATEWAY-NOTICE.txt"
cp "$root/crates/bitnet-gui/assets/fonts/LICENSE.txt" "$stage/portable/bitnet-$version/licenses/FONT-OFL.txt"
cp "$licenses/THIRD-PARTY-NOTICES.txt" "$licenses/THIRD-PARTY-NOTICES.json" "$stage/portable/bitnet-$version/licenses/"
python3 "$root/scripts/build-user-tar.py" "$stage/portable/bitnet-$version" "$out/bitnet-user-$version-linux-$arch.tar.gz"
set -- "$out/bitnet-core_${version}_${arch}.deb" "$out/bitnet-gui_${version}_${arch}.deb" "$out/bitnet-user-$version-linux-$arch.tar.gz"
if [ "${EXTRA_FORMATS:-0}" = 1 ]; then
    sh "$root/scripts/package-linux-extras.sh" "$stage" "$out" "$version" "$arch" "$glibc_min"
    case "$arch" in amd64) rpmarch=x86_64;; arm64) rpmarch=aarch64;; esac
    set -- "$@" "$out/BitNet-$version-linux-$arch.AppImage" "$out/bitnet-$version-1.$rpmarch.rpm"
fi
for asset do
    "$minisign" -S -s "$secret" -m "$asset" -x "$asset.minisig" >/dev/null 2>&1
    "$minisign" -V -p "$public" -m "$asset" -x "$asset.minisig" >/dev/null 2>&1
done
for asset do (cd "$out" && sha256sum "${asset##*/}"); done > "$out/SHA256SUMS"
printf 'Built packages in %s; nothing was installed.\n' "$out"
if [ -f "$bin/bitnet-build-source.json" ]; then cp "$bin/bitnet-build-source.json" "$out/build-source.json"; fi
python3 - "$out" "$version" "$arch" "$bin/bitnet-build-source.json" <<'PY'
import json, sys
from pathlib import Path
out, version, arch, record = sys.argv[1:5]
source_sha256 = json.loads(Path(record).read_text(encoding='utf-8'))['source_sha256']
info = dict(version=version, arch=arch, renderer='vello-cpu', source_sha256=source_sha256)
Path(out, 'build-info.json').write_text(json.dumps(info, indent=2) + '\n', encoding='utf-8')
PY
