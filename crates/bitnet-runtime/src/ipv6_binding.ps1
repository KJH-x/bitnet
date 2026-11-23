# The host supplies a validated adapter GUID, index, and closed action name.
$ErrorActionPreference = 'Stop'
$env:PSModulePath = [IO.Path]::Combine($PSHOME, 'Modules')
$bindingId = '{{{0}}}::ms_tcpip6' -f $interfaceGuid

function Get-TargetBinding {
    $bindings = @(Get-NetAdapterBinding -ComponentID ms_tcpip6 -IncludeHidden -AllBindings |
        Where-Object { $_.InstanceID -ieq $bindingId })
    if ($bindings.Count -ne 1) {
        throw 'IPv6 binding not found'
    }
    return $bindings[0]
}

function Set-TargetBinding([bool]$enabled) {
    $binding = Get-TargetBinding
    if ($binding.Enabled -ne $enabled) {
        Set-NetAdapterBinding -InputObject $binding -Enabled $enabled -Confirm:$false
    }
    for ($attempt = 0; $attempt -lt 8; $attempt++) {
        if ((Get-TargetBinding).Enabled -eq $enabled) {
            return
        }
        Start-Sleep -Milliseconds 250
    }
    throw 'IPv6 binding did not reach the requested state'
}

try {
    Import-Module -Name ([IO.Path]::Combine($PSHOME, 'Modules', 'NetAdapter', 'NetAdapter.psd1'))
    $mutex = [Threading.Mutex]::new($false, "Global\BitNet.IPv6.$interfaceGuid")
    $owned = $false
    try {
        try {
            $owned = $mutex.WaitOne(0)
        } catch [Threading.AbandonedMutexException] {
            $owned = $true
        }
        if (!$owned) {
            exit 3
        }
        $adapters = @(Get-NetAdapter -IncludeHidden | Where-Object {
            $_.ifIndex -eq $interfaceIndex -and ([string]$_.InterfaceGuid).Trim('{}') -ieq $interfaceGuid
        })
        if ($adapters.Count -ne 1) {
            exit 2
        }
        switch ($action) {
            'ipv6-enable'  { Set-TargetBinding $true }
            'ipv6-disable' { Set-TargetBinding $false }
            { $_ -eq 'ipv6-reset' -or $_ -eq 'ipv6-renew' } {
                if ($action -eq 'ipv6-renew') {
                    if ($adapters[0].Status -ne 'Up' -or !(Get-TargetBinding).Enabled) { exit 2 }
                    # ipconfig accepts patterns; only pass a single literal adapter alias.
                    $alias = [string]$adapters[0].Name
                    if ([string]::IsNullOrWhiteSpace($alias) -or $alias -match '[*?"/]|[\x00-\x1f]|^[- ]|[\\ ]$') { exit 2 }
                }
                try {
                    Set-TargetBinding $false
                } finally {
                    Set-TargetBinding $true
                }
                if ($action -eq 'ipv6-renew') {
                    $current = @(Get-NetAdapter -IncludeHidden | Where-Object {
                        $_.ifIndex -eq $interfaceIndex -and ([string]$_.InterfaceGuid).Trim('{}') -ieq $interfaceGuid
                    })
                    if ($current.Count -ne 1 -or $current[0].Name -cne $alias) { exit 2 }
                    & ([IO.Path]::Combine($PSHOME, '..', '..', 'ipconfig.exe')) /renew6 $alias | Out-Null
                    if ($LASTEXITCODE -ne 0) { exit 1 }
                }
            }
            default { exit 1 }
        }
    } finally {
        if ($owned) {
            $mutex.ReleaseMutex()
        }
        $mutex.Dispose()
    }
} catch {
    exit 1
}
exit 0
