param([Parameter(Mandatory)][string]$Directory,[Parameter(Mandatory)][string]$SecretKey,[Parameter(Mandatory)][string]$PublicKey,[string]$Minisign=$env:BITNET_MINISIGN)
$ErrorActionPreference='Stop'
if(-not $Minisign) {$Minisign=Join-Path $PSScriptRoot '..\.local\tools\minisign\minisign-win64\x86_64\minisign.exe'}
foreach($file in Get-ChildItem -LiteralPath $Directory -File | Where-Object {$_.Extension -ne '.minisig' -and $_.Name -ne 'build-info.json'}) {
    & $Minisign -S -s $SecretKey -m $file.FullName -x "$($file.FullName).minisig" *> $null
    if($LASTEXITCODE) {throw "Signing failed: $($file.Name)"}
    & $Minisign -V -p $PublicKey -m $file.FullName -x "$($file.FullName).minisig" *> $null
    if($LASTEXITCODE) {throw "Signature self-check failed: $($file.Name)"}
}
Write-Output 'Release signatures generated and self-checked; nothing was uploaded.'
