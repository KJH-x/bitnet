param([switch]$DotnetOnly)
$ErrorActionPreference='Stop'
$projectRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$toolRoot=Join-Path $projectRoot '.local\tools'
New-Item -ItemType Directory -Force -Path $toolRoot | Out-Null
function Get-CheckedArchive([string]$Url,[string]$Path,[string]$Hash,[string]$Algorithm='SHA256') {
    if (-not (Test-Path -LiteralPath $Path)) {
        & curl.exe --fail --location --retry 2 --max-time 300 --silent --show-error --output $Path $Url
        if($LASTEXITCODE) {throw "Tool download failed: $Url"}
    }
    if ((Get-FileHash -LiteralPath $Path -Algorithm $Algorithm).Hash -ne $Hash) {throw "Tool archive checksum mismatch: $Path"}
}
$sdkVersion='9.0.300'
$dotnet=Join-Path $toolRoot "dotnet-$sdkVersion\dotnet.exe"
if (-not (Test-Path -LiteralPath $dotnet)) {
    $metadata=Invoke-RestMethod 'https://builds.dotnet.microsoft.com/dotnet/release-metadata/9.0/releases.json' -TimeoutSec 45
    $sdk=@($metadata.releases | ForEach-Object {$_.sdks} | Where-Object {$_.version -eq $sdkVersion})[0]
    $asset=@($sdk.files | Where-Object {$_.rid -eq 'win-x64' -and $_.name -eq 'dotnet-sdk-win-x64.zip'})[0]
    if (-not $asset -or $asset.url -notlike 'https://builds.dotnet.microsoft.com/*') {throw 'Pinned .NET SDK was not found in Microsoft release metadata'}
    $archive=Join-Path $toolRoot "dotnet-sdk-$sdkVersion-win-x64.zip"
    Get-CheckedArchive $asset.url $archive $asset.hash 'SHA512'
    Expand-Archive -LiteralPath $archive -DestinationPath (Split-Path $dotnet) -Force
}
if (-not $DotnetOnly) {
    $nsis=Join-Path $toolRoot 'nsis-3.13\makensis.exe'
    if (-not (Test-Path -LiteralPath $nsis)) {
        $archive=Join-Path $toolRoot 'nsis-3.13.zip'
        Get-CheckedArchive 'https://master.dl.sourceforge.net/project/nsis/NSIS%203/3.13/nsis-3.13.zip?viasf=1' $archive 'ba63dffc4410ee89193e1cb5a41989991bd77c61068da17e3156d136b7b0b3d8'
        Expand-Archive -LiteralPath $archive -DestinationPath $toolRoot -Force
    }
    $minisign=Join-Path $toolRoot 'minisign\minisign-win64\x86_64\minisign.exe'
    if (-not (Test-Path $minisign)) {
        $archive=Join-Path $toolRoot 'minisign-0.12-win64.zip'
        Get-CheckedArchive 'https://github.com/jedisct1/minisign/releases/download/0.12/minisign-0.12-win64.zip' $archive '37b600344e20c19314b2e82813db2bfdcc408b77b876f7727889dbd46d539479'
        Expand-Archive -LiteralPath $archive -DestinationPath (Join-Path $toolRoot 'minisign') -Force
    }
}
Write-Output "Portable tools ready under $toolRoot (no system installation)"
