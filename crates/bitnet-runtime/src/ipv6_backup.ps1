# The administrator helper supplies a new directory under its protected backup root.
$ErrorActionPreference = 'Stop'
$env:PSModulePath = [IO.Path]::Combine($PSHOME, 'Modules')

try {
    $netsh = [IO.Path]::Combine($PSHOME, '..', '..', 'netsh.exe')
    $configuration = @(& $netsh interface ipv6 dump)
    if ($LASTEXITCODE -ne 0 -or ($configuration -join "`n") -notmatch '(?im)^\s*pushd\s+interface\s+ipv6\s*$') {
        exit 1
    }
    $adapters = @(Get-NetAdapter -IncludeHidden | Select-Object Name, ifIndex, InterfaceGuid,
        InterfaceDescription, Status, HardwareInterface)
    if (!$adapters.Count) { exit 1 }
    $addresses = @(Get-NetIPAddress -PolicyStore ActiveStore | Where-Object AddressFamily -eq IPv6 |
        Select-Object InterfaceIndex, InterfaceAlias, IPAddress, PrefixLength, PrefixOrigin,
            SuffixOrigin, AddressState, SkipAsSource, PreferredLifetime, ValidLifetime, PolicyStore)
    $staticAddresses = @(Get-NetIPAddress -PolicyStore PersistentStore |
        Where-Object AddressFamily -eq IPv6 | Select-Object InterfaceIndex, InterfaceAlias,
            IPAddress, PrefixLength, PrefixOrigin, SkipAsSource, PolicyStore)
    $routes = @(Get-NetRoute -PolicyStore PersistentStore | Where-Object AddressFamily -eq IPv6 |
        Select-Object InterfaceIndex, InterfaceAlias, DestinationPrefix, NextHop, RouteMetric,
            Protocol, Publish, PolicyStore)
    $ipInterfaces = @(Get-NetIPInterface -AddressFamily IPv6 | Select-Object InterfaceIndex,
        InterfaceAlias, Dhcp, RouterDiscovery, InterfaceMetric, NlMtu, DadTransmits)
    $dns = @(Get-DnsClientServerAddress -AddressFamily IPv6 | Select-Object InterfaceIndex,
        InterfaceAlias, ServerAddresses)

    $snapshot = [ordered]@{
        SchemaVersion = 1
        Adapters = $adapters
        IPv6Addresses = $addresses
        PersistentIPv6Addresses = $staticAddresses
        PersistentIPv6Routes = $routes
        IPv6Interfaces = $ipInterfaces
        IPv6DnsServers = $dns
    }
    $encoding = [Text.UTF8Encoding]::new($false)
    $scriptFile = [IO.Path]::Combine($backupDirectory, 'ipv6-netsh.txt')
    $dataFile = [IO.Path]::Combine($backupDirectory, 'ipv6-settings.json')
    $scriptText = $configuration -join "`r`n"
    $json = ConvertTo-Json $snapshot -Depth 6
    foreach ($entry in @(@{ Path = $scriptFile; Text = $scriptText }, @{ Path = $dataFile; Text = $json })) {
        $data = $encoding.GetBytes($entry.Text)
        if (!$data.Length -or $data.Length -gt 4MB) { exit 1 }
        $file = [IO.File]::Open($entry.Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write)
        try {
            $file.Write($data, 0, $data.Length)
            $file.Flush($true)
        } finally {
            $file.Dispose()
        }
        if ([IO.File]::ReadAllText($entry.Path, $encoding) -cne $entry.Text) { exit 1 }
    }
    $verified = ConvertFrom-Json ([IO.File]::ReadAllText($dataFile, $encoding))
    if ($verified.SchemaVersion -ne 1 -or $verified.Adapters.Count -ne $adapters.Count) { exit 1 }
} catch {
    exit 1
}
exit 0
