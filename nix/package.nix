{ lib, rustPlatform, pkg-config, makeWrapper, openssl, libsecret
, fontconfig, libxkbcommon, xorg, wayland, systemd }:

rustPlatform.buildRustPackage {
  pname = "bitnet";
  version = (builtins.fromTOML (builtins.readFile ../Cargo.toml)).workspace.package.version;
  src = lib.cleanSourceWith {
    src = lib.cleanSource ../.;
    filter = path: _: !(builtins.elem (baseNameOf path) [ ".local" "target" "build" "dist" ]);
  };
  cargoLock = {
    lockFile = ../Cargo.lock;
    outputHashes = {
      "cosmic-text-0.19.0" = "sha256-TJ5QXLPOxjoaelH/1TPD01OvTAXcow6bvgI4EVE18vs=";
      "iced-0.15.0-dev" = "sha256-nC7JZHcodgZu6uzkuaHvDpsj+cFUibgKOKgcR6UD6xg=";
      "winit-0.30.8" = "sha256-pQn1lCFSJMkjUfHoggEzMHnm5k+Chnzi5JEDjahnjUA=";
    };
  };
  nativeBuildInputs = [ pkg-config makeWrapper ];
  buildInputs = [ openssl fontconfig libxkbcommon xorg.libX11 wayland ];
  postPatch = ''
    substituteInPlace crates/bitnet-platform/src/credentials.rs \
      --replace-fail /usr/bin/secret-tool ${libsecret}/bin/secret-tool
    substituteInPlace crates/bitnet-platform/src/service.rs \
      --replace-fail /usr/bin/systemctl ${systemd}/bin/systemctl
  '';
  postInstall = ''
    install -Dm644 installers/linux/bitnet.desktop "$out/share/applications/bitnet.desktop"
    install -Dm644 installers/linux/bitnet.svg "$out/share/icons/hicolor/scalable/apps/bitnet.svg"
    install -Dm644 LICENSE "$out/share/licenses/bitnet/LICENSE"
    install -Dm644 crates/bitnet-gateway/NOTICE "$out/share/licenses/bitnet/GATEWAY-NOTICE.txt"
    wrapProgram "$out/bin/bitnet-gui" \
      --prefix LD_LIBRARY_PATH : "${lib.makeLibraryPath [ fontconfig libxkbcommon xorg.libX11 wayland ]}"
  '';
  meta = {
    description = "Desktop and command line clients for the BIT campus network";
    homepage = "https://github.com/KJH-x/bitnet";
    license = lib.licenses.mit;
    mainProgram = "bitnet-gui";
    platforms = [ "x86_64-linux" "aarch64-linux" ];
  };
}
