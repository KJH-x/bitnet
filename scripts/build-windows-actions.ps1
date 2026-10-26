param([ValidateSet('x86','x64','arm64')][string]$Arch='x64',[string]$VisualStudio)
$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if(-not $VisualStudio) {
    $vswhere=Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    $VisualStudio=(& $vswhere -latest -prerelease -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath | Select-Object -First 1)
}
if(-not $VisualStudio) {throw 'Visual Studio C++ tools are required'}
$version=(Get-Content (Join-Path $VisualStudio 'VC\Auxiliary\Build\Microsoft.VCToolsVersion.default.txt') -Raw).Trim()
$linker=Join-Path $VisualStudio "VC\Tools\MSVC\$version\bin\Hostx64\x64\link.exe"
$dumpbin=Join-Path $VisualStudio "VC\Tools\MSVC\$version\bin\Hostx64\x64\dumpbin.exe"
$sdkRoot=Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\Lib'
$sdk=Get-ChildItem -LiteralPath $sdkRoot -Directory | Sort-Object {[version]$_.Name} -Descending | Where-Object {Test-Path (Join-Path $_.FullName "um\$Arch\advapi32.lib")} | Select-Object -First 1
if(-not $sdk -or -not (Test-Path $linker)) {throw 'Windows SDK import libraries and an x64 MSVC linker are required'}
$out=Join-Path $root "build\windows-actions-$Arch"
New-Item -ItemType Directory -Force $out | Out-Null
$target=@{x86='i686-pc-windows-msvc';x64='x86_64-pc-windows-msvc';arm64='aarch64-pc-windows-msvc'}[$Arch]
& rustc "$root\installers\windows\uninstall_actions.rs" --edition 2021 --crate-name bitnet_uninstall_actions --crate-type cdylib --target $target -C opt-level=s -C panic=abort -C strip=symbols "-Clinker=$linker" '-L' "native=$($sdk.FullName)\um\$Arch" -C link-arg=/NOENTRY -C link-arg=/NODEFAULTLIB -o "$out\BitNetUninstallActions.dll"
if($LASTEXITCODE) {throw 'Native uninstall action build failed'}
$exports=(& $dumpbin /nologo /exports "$out\BitNetUninstallActions.dll") -join "`n"
if($LASTEXITCODE -or $exports -notmatch '(?m)\sRemoveStartup(?:\s+=|\s*$)') {throw 'Native action does not export the expected MSI entry point'}
