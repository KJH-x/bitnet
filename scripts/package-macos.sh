#!/bin/sh
# Build-only; pkgbuild/hdiutil never execute package postinstall scripts.
set -eu
test "$(uname -s)" = Darwin || { echo 'macOS packaging requires a macOS build host' >&2; exit 2; }
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
version=${VERSION:-0.1.0}; arch=${ARCH:-$(uname -m)}
case "$arch" in x86_64|aarch64|arm64) ;; *) exit 2;; esac
bin=${BIN_DIR:-$root/target/release}; stage=$root/build/macos-$arch; out=$root/dist/macos-$arch
python3 "$root/scripts/fingerprint-source.py" --check "$bin/bitnet-build-source.json" >/dev/null
python3 - "$root/Cargo.toml" "$version" <<'PY'
import re, sys
from pathlib import Path
match = re.search(r'(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"', Path(sys.argv[1]).read_text(encoding='utf-8'))
if not match or match[1] != sys.argv[2]:
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
grep -aF "$BITNET_RELEASE_PUBLIC_KEY" "$bin/bitmaint" >/dev/null || { echo 'Rebuild bitmaint with this public key' >&2; exit 2; }
test ! -e "$stage" || { echo "Stage already exists: $stage" >&2; exit 2; }
app=$stage/payload/Applications/BitNet.app
helpers=$stage/payload/Library/PrivilegedHelperTools
mkdir -p "$app/Contents/MacOS" "$helpers/BitNetTemplates" "$stage/payload/Library/LaunchDaemons" "$stage/scripts" "$stage/dmg" "$out"
target=$(case "$arch" in x86_64) printf x86_64-apple-darwin;; *) printf aarch64-apple-darwin;; esac)
licenses=$root/build/licenses/macos-$arch
python3 "$root/scripts/generate-notices.py" --target "$target" --output-dir "$licenses" --strict
for docs in "$app/Contents/Resources/licenses" "$helpers/BitNetLicenses"; do
    mkdir -p "$docs"
    install -m 0644 "$root/LICENSE" "$docs/LICENSE"
    install -m 0644 "$root/crates/bitnet-gateway/NOTICE" "$docs/GATEWAY-NOTICE.txt"
    install -m 0644 "$licenses/THIRD-PARTY-NOTICES.txt" "$docs/THIRD-PARTY-NOTICES.txt"
    install -m 0644 "$licenses/THIRD-PARTY-NOTICES.json" "$docs/THIRD-PARTY-NOTICES.json"
done
for name in bitnet-gui bitengine bitctl bitmaint; do install -m 0755 "$bin/$name" "$app/Contents/MacOS/$name"; done
install -m 0755 "$bin/bitengine" "$helpers/top.nsapi.bitnet.engine"
install -m 0755 "$bin/bitmaint" "$helpers/top.nsapi.bitnet.maint"
install -m 0755 "$bin/bitkeychain" "$helpers/top.nsapi.bitnet.keychain"
for label in top.nsapi.bitnet top.nsapi.bitnet.keychain; do
    plutil -lint "$root/installers/macos/$label.plist"
    install -m 0644 "$root/installers/macos/$label.plist" "$helpers/BitNetTemplates/$label.plist"
    install -m 0644 "$root/installers/macos/$label.plist" "$stage/payload/Library/LaunchDaemons/$label.plist"
done
install -m 0755 "$root/installers/macos/postinstall" "$stage/scripts/postinstall"
cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>top.nsapi.bitnet.gui</string><key>CFBundleName</key><string>BitNet</string>
<key>CFBundleExecutable</key><string>bitnet-gui</string><key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>$version</string><key>CFBundleVersion</key><string>$version</string>
<key>LSMinimumSystemVersion</key><string>11.0</string><key>NSHighResolutionCapable</key><true/>
</dict></plist>
if [ -n "${CODESIGN_IDENTITY:-}" ]; then
    for binary in "$app"/Contents/MacOS/* "$helpers"/top.nsapi.bitnet.*; do codesign --force --options runtime --timestamp --sign "$CODESIGN_IDENTITY" "$binary"; done
fi
mkdir -p "$stage/service-payload"
cp -R "$stage/payload/Library" "$stage/service-payload/Library"
service=$out/Service.pkg
pkgbuild --root "$stage/service-payload" --scripts "$stage/scripts" --identifier top.nsapi.bitnet.service --version "$version" --install-location / --ownership recommended "$service"
if [ -n "${INSTALLER_SIGN_IDENTITY:-}" ]; then productsign --sign "$INSTALLER_SIGN_IDENTITY" "$service" "$out/Service-signed.pkg"; mv "$out/Service-signed.pkg" "$service"; fi
"$minisign" -S -s "$secret" -m "$service" -x "$service.minisig" >/dev/null 2>&1
"$minisign" -V -p "$public" -m "$service" -x "$service.minisig" >/dev/null 2>&1
mkdir -p "$app/Contents/Resources/packages"
cp "$service" "$app/Contents/Resources/packages/Service.pkg"
cp "$service.minisig" "$app/Contents/Resources/packages/Service.pkg.minisig"
if [ -n "${CODESIGN_IDENTITY:-}" ]; then codesign --force --options runtime --timestamp --sign "$CODESIGN_IDENTITY" "$app"; fi
pkg=$out/BitNet-$version-macos-$arch.pkg
mkdir -p "$stage/client-payload"
cp -R "$stage/payload/Applications" "$stage/client-payload/Applications"
client=$out/Client.pkg
pkgbuild --root "$stage/client-payload" --identifier top.nsapi.bitnet.client --version "$version" --install-location / --ownership recommended "$client"
if [ -n "${INSTALLER_SIGN_IDENTITY:-}" ]; then productsign --sign "$INSTALLER_SIGN_IDENTITY" "$client" "$out/Client-signed.pkg"; mv "$out/Client-signed.pkg" "$client"; fi
# Explicit full installation is a product containing the same disjoint component receipts.
productbuild --package "$client" --package "$service" "$pkg"
if [ -n "${INSTALLER_SIGN_IDENTITY:-}" ]; then productsign --sign "$INSTALLER_SIGN_IDENTITY" "$pkg" "$out/BitNet-$version-signed.pkg"; mv "$out/BitNet-$version-signed.pkg" "$pkg"; fi
cp -R "$app" "$stage/dmg/BitNet.app"
cp "$service" "$stage/dmg/Install Background Service.pkg"
cp "$service.minisig" "$stage/dmg/Install Background Service.pkg.minisig"
ln -s /Applications "$stage/dmg/Applications"
hdiutil create -quiet -volname BitNet -srcfolder "$stage/dmg" -format UDZO "$out/BitNet-$version-macos-$arch.dmg"
for asset in "$client" "$pkg" "$out/BitNet-$version-macos-$arch.dmg"; do
    "$minisign" -S -s "$secret" -m "$asset" -x "$asset.minisig" >/dev/null 2>&1
    "$minisign" -V -p "$public" -m "$asset" -x "$asset.minisig" >/dev/null 2>&1
done
(cd "$out" && shasum -a 256 ./*.pkg ./*.dmg > SHA256SUMS)
if [ -f "$bin/bitnet-build-source.json" ]; then cp "$bin/bitnet-build-source.json" "$out/build-source.json"; fi
printf 'Built and project-signature-checked %s. Apple Gatekeeper may require manual approval.\n' "$out"
