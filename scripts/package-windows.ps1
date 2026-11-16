param(
    [ValidateSet('x86','x64','arm64')][string]$Arch='x64',
    [string]$Version='0.1.2', [switch]$Development,
    [string]$SecretKey=$env:BITNET_RELEASE_SECRET_KEY_FILE,
    [string]$PublicKey=$env:BITNET_RELEASE_PUBLIC_KEY_FILE,
    [string]$Wix=$env:BITNET_WIX,
    [string]$Minisign=$env:BITNET_MINISIGN,
    [string]$MakeNsis=$env:BITNET_MAKENSIS,
    [switch]$SkipBuild,
    [ValidatePattern('^[a-z0-9-]+$')][string]$OutputSuffix
)
$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$sourceVersion=[regex]::Match((Get-Content "$root\Cargo.toml" -Raw),'(?ms)\[workspace\.package\].*?^version\s*=\s*"([^"]+)"').Groups[1].Value
if($sourceVersion -ne $Version) {throw 'Package version must match workspace.package.version; update source before building a release'}
$tools=Join-Path $root '.local\tools'
if(-not $Minisign) {$Minisign=Join-Path $tools 'minisign\minisign-win64\x86_64\minisign.exe'}
if(-not $MakeNsis) {$MakeNsis=Join-Path $tools 'nsis-3.13\makensis.exe'}
if(-not $Wix) {$Wix=Join-Path $tools 'wix-msi-source\wix.dll'}
foreach($tool in @($Minisign,$MakeNsis,$Wix)) {if(-not (Test-Path -LiteralPath $tool)) {throw "Build tool missing: $tool"}}
if($Development) {
    $keys=Join-Path $root '.local\release-keys'
    New-Item -ItemType Directory -Force $keys | Out-Null
    $identity=[Security.Principal.WindowsIdentity]::GetCurrent().User
    & "$env:SystemRoot\System32\icacls.exe" $keys '/inheritance:r' '/grant:r' "*$($identity.Value):(OI)(CI)F" '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' *> $null
    if($LASTEXITCODE) {throw 'Cannot protect the development signing-key directory'}
    $SecretKey=Join-Path $keys 'development.key'; $PublicKey=Join-Path $keys 'development.pub'
    if(-not (Test-Path $SecretKey)) {
        & $Minisign -G -W -s $SecretKey -p $PublicKey *> $null
        if($LASTEXITCODE) {throw 'Development signing key generation failed'}
    }
}
if(-not $SecretKey -or -not $PublicKey) {throw 'Provide release key files or explicitly use -Development'}
if(-not (Test-Path $SecretKey) -or -not (Test-Path $PublicKey)) {throw 'Signing key file is missing'}
$publicLines=@(Get-Content -LiteralPath $PublicKey | Where-Object {$_ -and -not $_.StartsWith('untrusted comment:')} | ForEach-Object {[string]$_})
if($publicLines.Count -ne 1) {throw 'Expected a Minisign public-key file'}
$env:BITNET_RELEASE_PUBLIC_KEY=$publicLines[0]
$triple=@{x86='i686-pc-windows-msvc';x64='x86_64-pc-windows-msvc';arm64='aarch64-pc-windows-msvc'}[$Arch]
$sourceRecord=Join-Path $root ".local\source-before-$Arch.json"
$fingerprint=(& python "$PSScriptRoot\fingerprint-source.py" --write $sourceRecord) | ConvertFrom-Json
if($LASTEXITCODE) {throw 'Cannot fingerprint application source'}
if(-not $SkipBuild) {
    Push-Location $root
    try {
        & cargo build --release --target $triple -p bitnet-runtime -p bitnet-cli -p bitnet-gui --bins
        if($LASTEXITCODE) {throw 'Release binary build failed'}
    } finally {Pop-Location}
    & python "$PSScriptRoot\fingerprint-source.py" --check $sourceRecord --write "$root\target\$triple\release\bitnet-build-source.json" | Out-Null
    if($LASTEXITCODE) {throw 'Source changed while compiling release binaries'}
} else {
    & python "$PSScriptRoot\fingerprint-source.py" --check "$root\target\$triple\release\bitnet-build-source.json" | Out-Null
    if($LASTEXITCODE) {throw 'Existing binaries are not attested to the current source; rebuild without -SkipBuild'}
}
$stage=Join-Path $root "target\$triple\release"
foreach($name in @('bitengine','bitctl','bitmaint','bitnet-gui')) {if(-not (Test-Path "$stage\$name.exe")) {throw "Missing release binary: $name"}}
& "$PSScriptRoot\check-windows-dependencies.ps1" -Directory $stage
if(-not [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes("$stage\bitmaint.exe")).Contains($publicLines[0])) {
    throw 'bitmaint was not compiled with this release public key; rebuild without -SkipBuild'
}
$outName=if($OutputSuffix) {"windows-$Arch-$OutputSuffix"} else {"windows-$Arch"}
$out=Join-Path $root "dist\$outName"
New-Item -ItemType Directory -Force $out | Out-Null
$licenses=Join-Path $root "build\licenses\windows-$Arch"
& python "$PSScriptRoot\generate-notices.py" --target $triple --output-dir $licenses --strict
if($LASTEXITCODE) {throw 'Third-party notices generation failed'}
& "$PSScriptRoot\build-windows-actions.ps1" -Arch $Arch
function WixBuild([string]$Source,[string]$Output,[string[]]$Definitions) {
    $args=@('build',$Source,'-arch',$Arch,'-o',$Output)+$Definitions
    if($Wix.EndsWith('.dll')) {& "$tools\dotnet-9.0.300\dotnet.exe" $Wix @args} else {& $Wix @args}
    if($LASTEXITCODE) {throw "MSI build failed: $Source"}
}
$service=Join-Path $out 'Service.msi'
$client=Join-Path $out 'Client.msi'
$hash=[Security.Cryptography.SHA1]::Create().ComputeHash([Text.Encoding]::Unicode.GetBytes('BITNET'))
$parts=0..4 | ForEach-Object {[BitConverter]::ToUInt32($hash,$_ * 4)}
$sid='S-1-5-80-'+($parts -join '-')
$sddl="D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;$sid)"
$programFiles=if($Arch -eq 'x86') {'ProgramFilesFolder'} else {'ProgramFiles64Folder'}
$definitions=@('-d',"Version=$Version",'-d',"Stage=$stage")
function ProductCode([string]$Kind) {
    $bytes=[Security.Cryptography.SHA256]::Create().ComputeHash([Text.Encoding]::UTF8.GetBytes("top.nsapi.bitnet/$Kind/$Version/$Arch"))
    $guidBytes=New-Object byte[] 16; [Array]::Copy($bytes,$guidBytes,16)
    return [Guid]::new($guidBytes).ToString('B').ToUpperInvariant()
}
$definitions+=@('-d',"ClientProductCode=$(ProductCode 'client')",'-d',"ServiceProductCode=$(ProductCode 'service')")
$definitions+=@('-d',"UninstallActions=$root\build\windows-actions-$Arch\BitNetUninstallActions.dll")
$definitions+=@('-d',"ProjectLicense=$root\LICENSE",'-d',"GatewayNotice=$root\crates\bitnet-gateway\NOTICE",'-d',"ThirdPartyNotices=$licenses\THIRD-PARTY-NOTICES.txt",'-d',"ThirdPartyMetadata=$licenses\THIRD-PARTY-NOTICES.json")
WixBuild "$root\installers\windows\Service.wxs" $service ($definitions+@('-d',"ProgramFilesId=$programFiles",'-d',"ServiceDataSddl=$sddl"))
& $Minisign -S -s $SecretKey -m $service -x "$service.minisig" -t "BitNet $Version Windows $Arch service" *> $null
if($LASTEXITCODE) {throw 'Service MSI signing failed'}
& $Minisign -V -p $PublicKey -m $service -x "$service.minisig" *> $null
if($LASTEXITCODE) {throw 'Service MSI signature self-check failed'}
WixBuild "$root\installers\windows\Client.wxs" $client ($definitions+@('-d',"ServiceMsi=$service",'-d',"ServiceSignature=$service.minisig"))
& "$PSScriptRoot\inspect-windows-packages.ps1" -Directory $out
$setup=Join-Path $out "BitNetSetup-$Version-windows-$Arch.exe"
& $MakeNsis /INPUTCHARSET UTF8 "/DCLIENT_MSI=$client" "/DOUTPUT=$setup" "$root\installers\windows\Setup.nsi"
if($LASTEXITCODE) {throw 'NSIS wrapper build failed'}
$portable=Join-Path $root "build\portable-windows-$Arch-$([Guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Force "$portable\licenses","$portable\packages" | Out-Null
foreach($name in @('bitengine','bitctl','bitmaint','bitnet-gui')) {Copy-Item "$stage\$name.exe" $portable}
Copy-Item "$root\LICENSE" "$portable\licenses\LICENSE"
Copy-Item "$root\crates\bitnet-gateway\NOTICE" "$portable\licenses\GATEWAY-NOTICE.txt"
Copy-Item "$licenses\THIRD-PARTY-NOTICES.*" "$portable\licenses"
Copy-Item $service "$portable\packages\Service.msi"
Copy-Item "$service.minisig" "$portable\packages\Service.msi.minisig"
$zip=Join-Path $out "BitNet-$Version-windows-$Arch.zip"
Compress-Archive -Path "$portable\*" -DestinationPath $zip -CompressionLevel Optimal -Force
$portableExe=Join-Path $out "BitNetPortable-$Version-windows-$Arch.exe"
$payloadHashes=@('bitnet-gui','bitengine','bitctl','bitmaint') | ForEach-Object {
    (Get-FileHash -LiteralPath "$portable\$_.exe" -Algorithm SHA256).Hash
}
$payloadBytes=[Text.Encoding]::ASCII.GetBytes(($payloadHashes -join ':'))
$payloadHash=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($payloadBytes))
& $MakeNsis /INPUTCHARSET UTF8 "/DSTAGE=$portable" "/DOUTPUT=$portableExe" "/DPAYLOAD_ID=$Version-$Arch-$payloadHash" "$root\installers\windows\Portable.nsi"
if($LASTEXITCODE) {throw 'NSIS portable wrapper build failed'}
foreach($asset in @($client,$setup,$zip,$portableExe)) {
    & $Minisign -S -s $SecretKey -m $asset -x "$asset.minisig" -t "BitNet $Version Windows $Arch" *> $null
    if($LASTEXITCODE) {throw 'Release artifact signing failed'}
    & $Minisign -V -p $PublicKey -m $asset -x "$asset.minisig" *> $null
    if($LASTEXITCODE) {throw 'Release artifact signature self-check failed'}
}
@($service,$client,$setup,$zip,$portableExe) | ForEach-Object {$item=Get-Item $_; "{0}  {1}" -f (Get-FileHash $_ -Algorithm SHA256).Hash.ToLowerInvariant(),$item.Name} | Set-Content -Encoding ascii "$out\SHA256SUMS"
@{version=$Version;arch=$Arch;development=[bool]$Development;public_key=$publicLines[0];source_sha256=$fingerprint.source_sha256;wix_source='7.0.0/b8977d6f88e7b68e000bac226a2814f236770570';static_crt_imports_checked=$true;installation_tested=$false} | ConvertTo-Json | Set-Content -Encoding utf8 "$out\build-info.json"
Write-Output "Built and signature-checked Windows packages: $out. Nothing was installed."
