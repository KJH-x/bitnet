$ErrorActionPreference = 'Stop'
$bin = $PSScriptRoot
$dir = Join-Path $bin ('.preview-data\' + [Guid]::NewGuid().ToString('N'))
$oldData = $env:BITNET_DATA_DIR
$oldSimulate = $env:BITNET_SIMULATE_ONLY
$ctl = Join-Path $bin 'bitctl.exe'
$started = $false
try {
    $env:BITNET_DATA_DIR = $dir
    $env:BITNET_SIMULATE_ONLY = '1'
    & $ctl --scope user --data-dir $dir start --simulate
    if ($LASTEXITCODE -ne 0) { throw 'Cannot start the simulation engine.' }
    $started = $true
    Write-Host 'Use the BitNet tray menu to quit the preview. Closing the window hides it.'
    Start-Process -FilePath (Join-Path $bin 'bitnet-gui.exe') -WorkingDirectory $bin -Wait
} finally {
    if ($started) {
        & $ctl --scope user --data-dir $dir stop | Out-Null
    }
    $env:BITNET_DATA_DIR = $oldData
    $env:BITNET_SIMULATE_ONLY = $oldSimulate
}
