#!/bin/sh
# Stage an AppImage and a desktop-only RPM from an attested Linux payload.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
test "$#" -eq 5 || { echo 'Usage: package-linux-extras.sh STAGE OUT VERSION ARCH GLIBC_MIN' >&2; exit 2; }
stage=$1 out=$2 version=$3 arch=$4 glibc_min=$5
case "$arch" in amd64) native=x86_64;; arm64) native=aarch64;; *) echo 'AppImage and RPM support amd64 and arm64' >&2; exit 2;; esac
test "$(uname -m)" = "$native" || { echo 'AppImage and RPM require a native build host' >&2; exit 2; }
case "$version:$glibc_min" in *[!0-9.:]*) echo 'Invalid package version' >&2; exit 2;; esac
runtime=${APPIMAGE_RUNTIME:?Path to a trusted AppImage runtime is required}
test -s "$runtime"
test "$(od -An -tx1 -j8 -N3 "$runtime" | tr -d ' \n')" = 414902
command -v mksquashfs >/dev/null
command -v rpmbuild >/dev/null

mkdir -p "$root/build"
work=$(mktemp -d "$root/build/linux-extra-$arch.XXXXXXXX")
app=$work/BitNet.AppDir
payload=$stage/portable/bitnet-$version
mkdir -p "$app/usr/lib/bitnet/licenses"
install -m 0755 "$root/installers/linux/portable-launch.sh" "$app/AppRun"
install -m 0644 "$root/installers/linux/bitnet.desktop" "$app/bitnet.desktop"
install -m 0644 "$root/installers/linux/bitnet.svg" "$app/bitnet.svg"
install -m 0644 "$payload/LICENSE" "$app/usr/lib/bitnet/licenses/LICENSE"
install -m 0644 "$payload"/licenses/* "$app/usr/lib/bitnet/licenses/"
install -m 0644 "$root/installers/linux/appimage-runtime.LICENSE" \
    "$app/usr/lib/bitnet/licenses/APPIMAGE-RUNTIME.txt"
for program in bitengine bitctl bitmaint bitnet-gui bitnet-tui bitnet-web; do
    install -m 0755 "$payload/$program" "$app/usr/lib/bitnet/$program"
done
(cd "$app/usr/lib/bitnet" && sha256sum bitengine bitctl bitmaint bitnet-gui bitnet-tui bitnet-web > SHA256SUMS)
# A type-2 image consists of the runtime ELF followed by the SquashFS.
mksquashfs "$app" "$work/payload.squashfs" -noappend -all-root -no-xattrs -comp zstd -processors 2
cat "$runtime" "$work/payload.squashfs" > "$work/BitNet.AppImage"
chmod 0755 "$work/BitNet.AppImage"
test "$("$work/BitNet.AppImage" --appimage-offset)" = "$(wc -c < "$runtime" | tr -d ' ')"
install -m 0755 "$work/BitNet.AppImage" "$out/BitNet-$version-linux-$arch.AppImage"

rpm=$work/rpm
mkdir -p "$rpm/SOURCES" "$work/tmp"
tarball=bitnet-user-$version-linux-$arch.tar.gz
install -m 0644 "$out/$tarball" "$rpm/SOURCES/$tarball"
install -m 0644 "$root/installers/linux/bitnet.desktop" "$rpm/SOURCES/bitnet.desktop"
install -m 0644 "$root/installers/linux/bitnet.svg" "$rpm/SOURCES/bitnet.svg"
rpmbuild -bb --define "_topdir $rpm" --define "_tmppath $work/tmp" --define 'dist %{nil}' \
    --define "bitnet_source $tarball" --define "bitnet_version $version" \
    --define "bitnet_glibc $glibc_min" "$root/installers/linux/bitnet.spec"
install -m 0644 "$rpm/RPMS/$native/bitnet-$version-1.$native.rpm" "$out/"
