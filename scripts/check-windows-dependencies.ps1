param([Parameter(Mandatory)][string]$Directory)
$ErrorActionPreference='Stop'
$vswhere=Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$vs=(& $vswhere -latest -prerelease -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath | Select-Object -First 1)
$version=(Get-Content (Join-Path $vs 'VC\Auxiliary\Build\Microsoft.VCToolsVersion.default.txt') -Raw).Trim()
$dumpbin=Join-Path $vs "VC\Tools\MSVC\$version\bin\Hostx64\x64\dumpbin.exe"
foreach($name in @('bitengine','bitctl','bitmaint','bitnet-gui','bitnet-tui','bitnet-web')) {
    $imports=(& $dumpbin /nologo /dependents (Join-Path $Directory "$name.exe")) -join "`n"
    if($LASTEXITCODE) {throw "Cannot inspect imports: $name"}
    if($imports -match '(?i)\b(?:VCRUNTIME|MSVCP)\w*\.dll|api-ms-win-crt-') {throw "External MSVC/UCRT runtime dependency remains in $name"}
}
Write-Output 'Application binaries import no separately installed MSVC/UCRT runtime.'
