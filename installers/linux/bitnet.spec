%global debug_package %{nil}
%global _build_id_links none
Name: bitnet
Version: %{bitnet_version}
Release: 1%{?dist}
Summary: BIT campus network desktop client and personal engine
License: MIT
URL: https://github.com/KJH-x/bitnet
Source0: %{bitnet_source}
Source1: bitnet.desktop
Source2: bitnet.svg
Requires: glibc >= %{bitnet_glibc}
Requires: libX11, libxkbcommon, fontconfig, libwayland-client
Recommends: libsecret
Conflicts: bitnet-core, bitnet-gui

%description
BitNet connects to the BIT campus network using a separate personal engine,
with a desktop client and a command-line interface.
The package includes a terminal interface and an on-demand web log viewer.

%prep
%setup -q

%install
install -d "%{buildroot}%{_bindir}" "%{buildroot}%{_datadir}/applications" \
    "%{buildroot}%{_datadir}/icons/hicolor/scalable/apps" "%{buildroot}%{_docdir}/bitnet"
for program in bitengine bitctl bitmaint bitnet-gui bitnet-tui bitnet-web; do
    install -m 0755 "$program" "%{buildroot}%{_bindir}/$program"
done
install -m 0644 "%{SOURCE1}" "%{buildroot}%{_datadir}/applications/bitnet.desktop"
install -m 0644 "%{SOURCE2}" "%{buildroot}%{_datadir}/icons/hicolor/scalable/apps/bitnet.svg"
install -m 0644 licenses/* "%{buildroot}%{_docdir}/bitnet"

%files
%license LICENSE
%doc %{_docdir}/bitnet/
%{_bindir}/bitengine
%{_bindir}/bitctl
%{_bindir}/bitmaint
%{_bindir}/bitnet-gui
%{_bindir}/bitnet-tui
%{_bindir}/bitnet-web
%{_datadir}/applications/bitnet.desktop
%{_datadir}/icons/hicolor/scalable/apps/bitnet.svg
