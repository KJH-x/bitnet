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

function Get-TargetAdapter {
    $matches = @(Get-NetAdapter -IncludeHidden | Where-Object {
        $_.ifIndex -eq $interfaceIndex -and ([string]$_.InterfaceGuid).Trim('{}') -ieq $interfaceGuid
    })
    if ($matches.Count -ne 1) {
        exit 2
    }
    return $matches[0]
}

function Get-QualifiedAdapter {
    $adapter = Get-TargetAdapter
    $binding = Get-TargetBinding
    $prefix = '{{{0}}}::' -f $interfaceGuid
    $bindings = @(Get-NetAdapterBinding -IncludeHidden -AllBindings | Where-Object {
        ([string]$_.InstanceID).StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)
    })
    $ipv4 = @($bindings | Where-Object { $_.ComponentID -eq 'ms_tcpip' -and $_.Enabled }).Count -gt 0
    $uplink = @($bindings | Where-Object { $_.ComponentID -eq 'vms_pp' -and $_.Enabled }).Count -gt 0
    if ($adapter.HardwareInterface -and $adapter.InterfaceType -in @(6, 71) -and
        (!$uplink -or $ipv4 -or $binding.Enabled)) {
        return $adapter
    }
    # Require the Windows Hyper-V management miniport and a host IP binding.
    if ($adapter.Virtual -and $adapter.InterfaceType -eq 6 -and
        $adapter.InterfaceDescription -like 'Hyper-V Virtual Ethernet Adapter*' -and
        ($ipv4 -or $binding.Enabled)) {
        return $adapter
    }
    exit 2
}

function Get-LiteralAlias($adapter) {
    # ipconfig accepts patterns; only pass a single literal adapter alias.
    $alias = [string]$adapter.Name
    if ([string]::IsNullOrWhiteSpace($alias) -or $alias -match '[*?"/]|[\x00-\x1f]|^[- ]|[\\ ]$') {
        exit 2
    }
    return $alias
}

function Invoke-Renew([string]$alias) {
    if ((Get-QualifiedAdapter).Name -cne $alias) { exit 2 }
    & ([IO.Path]::Combine($PSHOME, '..', '..', 'ipconfig.exe')) /renew6 $alias | Out-Null
    if ($LASTEXITCODE -ne 0) { exit 1 }
}

function Test-Dhcpv6 {
    $identities = @(Get-NetIPInterface -AddressFamily IPv6 -ErrorAction Stop |
        Where-Object InterfaceIndex -eq $interfaceIndex)
    if ($identities.Count -ne 1 -or $identities[0].Dhcp -ne 'Enabled') {
        return $false
    }
    if ($identities[0].ManagedAddressConfiguration -eq 'Enabled') {
        return $true
    }
    return @(Get-NetIPAddress -AddressFamily IPv6 -ErrorAction Stop | Where-Object {
        $_.InterfaceIndex -eq $interfaceIndex -and
            ($_.PrefixOrigin -eq 'Dhcp' -or $_.SuffixOrigin -eq 'Dhcp')
    }).Count -gt 0
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
        $adapter = Get-QualifiedAdapter
        switch ($action) {
            'ipv6-enable'  { Set-TargetBinding $true }
            'ipv6-disable' { Set-TargetBinding $false }
            { $_ -eq 'ipv6-reset' -or $_ -eq 'ipv6-renew' } {
                if ($action -eq 'ipv6-renew') {
                    if ($adapter.Status -ne 'Up' -or !(Get-TargetBinding).Enabled) { exit 2 }
                    $alias = Get-LiteralAlias $adapter
                }
                try {
                    Set-TargetBinding $false
                } finally {
                    Set-TargetBinding $true
                }
                if ($action -eq 'ipv6-renew') {
                    Invoke-Renew $alias
                }
            }
            'ipv6-renew-dhcp' {
                if ($adapter.Status -ne 'Up' -or !(Get-TargetBinding).Enabled -or !(Test-Dhcpv6)) { exit 2 }
                Invoke-Renew (Get-LiteralAlias $adapter)
            }
            'ipv6-restart-adapter' {
                if ($adapter.Status -ne 'Up') { exit 2 }
                try {
                    Restart-NetAdapter -InputObject $adapter -Confirm:$false
                } finally {
                    $current = Get-TargetAdapter
                    if ($current.Status -eq 'Disabled') {
                        Enable-NetAdapter -InputObject $current -Confirm:$false
                    }
                }
                if ((Get-TargetAdapter).Status -eq 'Disabled') { exit 1 }
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
