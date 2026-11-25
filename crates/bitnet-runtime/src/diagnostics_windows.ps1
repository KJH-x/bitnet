$ErrorActionPreference = 'SilentlyContinue'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)

function Text($Value) {
    if ($null -eq $Value -or "$Value" -eq '') { return '—' }
    return "$Value"
}

function Format-Endpoint([string]$Address, [int]$Port) {
    if ($Address.Contains(':')) { return "[${Address}]:$Port" }
    return "${Address}:$Port"
}

function ProxyEndpoints([string]$Value) {
    $items = @()
    foreach ($entry in ($Value -split '[;\s]+' | Select-Object -First 12)) {
        if (!$entry) { continue }
        $address = ($entry -split '=', 2)[-1]
        if ($address -notmatch '^\w+://') { $address = "http://$address" }
        $parsed = $null
        if ([Uri]::TryCreate($address, [UriKind]::Absolute, [ref]$parsed) -and $parsed.Host) {
            $items += "$($parsed.Host):$($parsed.Port)"
        }
    }
    return (Text ($items -join ', '))
}

$result = [ordered]@{ adapters = @(); routes = @(); egress = @(); proxies = @()
    processes = @(); sockets = @(); relations = @(); interception = @()
    reachability = @(); controllers = @()
    ipv6 = [ordered]@{ adapters = @(); addresses = @(); routes = @(); disabled_components = $null }
    unavailable = @() }

$adapters = @(Get-NetAdapter -IncludeHidden -ErrorAction SilentlyContinue | Select-Object -First 128)
$interfaces = @(Get-NetIPInterface -ErrorAction SilentlyContinue)
$addresses = @(Get-NetIPAddress -ErrorAction SilentlyContinue)
$bindings = @{}
Get-NetAdapterBinding -IncludeHidden -AllBindings -ErrorAction SilentlyContinue | ForEach-Object {
    if ($_.ComponentID -in @('ms_tcpip6', 'ms_tcpip', 'vms_pp') -and $_.InstanceID) {
        $bindings[[string]$_.InstanceID] = [bool]$_.Enabled
    }
}
$disabled = Get-ItemPropertyValue 'HKLM:\SYSTEM\CurrentControlSet\Services\Tcpip6\Parameters' DisabledComponents
if ($null -ne $disabled) {
    $result.ipv6.disabled_components = ([int64]$disabled -band 4294967295)
}
foreach ($adapter in $adapters) {
    $index = [uint32]$adapter.ifIndex
    if ($ipv6DiagnosticResampleIndex -and $index -ne $ipv6DiagnosticResampleIndex) { continue }
    $guid = '{' + "$($adapter.InterfaceGuid)".Trim('{}') + '}'
    $properties = $interfaces | Where-Object { $_.InterfaceIndex -eq $index -and $_.AddressFamily -eq 'IPv6' } |
        Select-Object -First 1
    $binding = $bindings["${guid}::ms_tcpip6"]
    $v4Bound = $bindings["${guid}::ms_tcpip"]
    $switchPort = $bindings["${guid}::vms_pp"] -and !$v4Bound -and !$binding
    $physical = $adapter.HardwareInterface -and $adapter.InterfaceType -in @(6, 71)
    $hyperV = $adapter.InterfaceDescription -like 'Hyper-V Virtual Ethernet Adapter*' -and ($v4Bound -or $binding)
    $result.ipv6.adapters += [ordered]@{
        index = $index
        guid = $guid
        name = "$($adapter.Name)"
        link_up = ($adapter.Status -eq 'Up')
        repair_eligible = [bool](($physical -or $hyperV) -and !$switchPort)
        binding = $binding
        dhcp = "$($properties.Dhcp)"
        managed_address = "$($properties.ManagedAddressConfiguration)"
        router_discovery = "$($properties.RouterDiscovery)"
    }
}
$result.ipv6.addresses = @($addresses | Where-Object {
    $_.AddressFamily -eq 'IPv6' -and (!$ipv6DiagnosticResampleIndex -or $_.InterfaceIndex -eq $ipv6DiagnosticResampleIndex)
} | Select-Object -First 256 | ForEach-Object {
    [ordered]@{
        index = [uint32]$_.InterfaceIndex
        address = ("$($_.IPAddress)" -split '%')[0]
        state = "$($_.AddressState)"
        prefix_origin = "$($_.PrefixOrigin)"
        suffix_origin = "$($_.SuffixOrigin)"
        skip_as_source = [bool]$_.SkipAsSource
        preferred_lifetime = "$($_.PreferredLifetime)"
        valid_lifetime = "$($_.ValidLifetime)"
    }
})
if ($ipv6DiagnosticResampleIndex) {
    $result | ConvertTo-Json -Depth 5 -Compress
    exit 0
}
$resolvers = @(Get-DnsClientServerAddress -ErrorAction SilentlyContinue)
$routes = @(Get-NetRoute -ErrorAction SilentlyContinue | Where-Object {
    $_.DestinationPrefix -in @('0.0.0.0/0', '0.0.0.0/1', '128.0.0.0/1', '::/0', '::/1', '8000::/1')
} | Select-Object -First 96)
$processes = @{}
$proxyPattern = '(?i)clash|mihomo|tun2socks|wireguard|tailscale|openvpn|zerotier|sing.box|trojan|shadowsocks|v2ray|xray|windivert'
foreach ($process in (Get-Process -ErrorAction SilentlyContinue)) {
    $processes[[int]$process.Id] = $process.ProcessName
    if ($process.ProcessName -match $proxyPattern -and $result.processes.Count -lt 64) {
        $result.processes += ,@("$($process.Id)", $process.ProcessName, 'name match')
    }
}
$byAddress = @{}
foreach ($adapter in $adapters) {
    $n = [int]$adapter.ifIndex
    $adapterAddresses = @($addresses | Where-Object InterfaceIndex -eq $n)
    foreach ($ip in $adapterAddresses.IPAddress) {
        if ($ip) { $byAddress["$ip"] = "$($adapter.Name) ($n)" }
    }
    $v4 = (($adapterAddresses | Where-Object AddressFamily -eq 'IPv4' | ForEach-Object { "$($_.IPAddress)/$($_.PrefixLength)" }) -join ', ')
    $v6 = (($adapterAddresses | Where-Object AddressFamily -eq 'IPv6' | ForEach-Object { "$($_.IPAddress)/$($_.PrefixLength)" }) -join ', ')
    $gateway = (($routes | Where-Object {
        $_.InterfaceIndex -eq $n -and $_.NextHop -notin @('0.0.0.0', '::')
    } | Select-Object -ExpandProperty NextHop -Unique) -join ', ')
    $dns = (($resolvers | Where-Object InterfaceIndex -eq $n | ForEach-Object { $_.ServerAddresses }) -join ', ')
    $result.adapters += ,@("$n", (Text $adapter.Name), (Text $adapter.Status),
        $(if ($adapter.HardwareInterface) { 'physical' } else { 'virtual' }),
        (Text $adapter.LinkSpeed), (Text $adapter.InterfaceDescription),
        (Text $v4), (Text $v6), (Text $gateway), (Text $dns))
}
if (!$adapters.Count) { $result.unavailable += 'adapters' }

foreach ($route in $routes) {
    $metric = $interfaces | Where-Object {
        $_.InterfaceIndex -eq $route.InterfaceIndex -and $_.AddressFamily -eq $route.AddressFamily
    } | Select-Object -ExpandProperty InterfaceMetric -First 1
    $effective = if ($null -ne $metric) { [int]$route.RouteMetric + [int]$metric } else { '—' }
    $result.routes += ,@("$($route.DestinationPrefix)", "$($route.InterfaceIndex)", (Text $route.InterfaceAlias),
        (Text $route.NextHop), "$($route.RouteMetric)", (Text $metric), "$effective")
}

$campus = @('10.0.0.55', '10.0.0.54', '10.0.32.32', '10.0.32.33')
$targets = @('223.5.5.5', '2400:3200::1') + $campus
foreach ($target in $targets) {
    $found = @(Find-NetRoute -RemoteIPAddress $target -ErrorAction SilentlyContinue)
    $route = $found | Where-Object NextHop | Select-Object -First 1
    $source = $found | Where-Object IPAddress | Select-Object -First 1
    $result.egress += ,@($target, (Text $route.DestinationPrefix), (Text $route.InterfaceIndex),
        (Text $route.InterfaceAlias), (Text $source.IPAddress), (Text $route.NextHop))
}

foreach ($target in ($ipv6DiagnosticTargets | Select-Object -First 16)) {
    $found = @(Find-NetRoute -RemoteIPAddress $target -ErrorAction SilentlyContinue)
    $route = $found | Where-Object DestinationPrefix | Select-Object -First 1
    $source = $found | Where-Object IPAddress | Select-Object -First 1
    $nextHop = if ($route.NextHop) { ("$($route.NextHop)" -split '%')[0] }
    $neighbor = if ($route -and $nextHop -and $nextHop -notin @('0.0.0.0', '::')) {
        Get-NetNeighbor -InterfaceIndex $route.InterfaceIndex -IPAddress $nextHop -ErrorAction SilentlyContinue |
            Select-Object -ExpandProperty State -First 1
    }
    $routeIndex = if ($route) { [uint32]$route.InterfaceIndex }
    $sourceIp = if ($source.IPAddress) { ("$($source.IPAddress)" -split '%')[0] }
    $result.ipv6.routes += [ordered]@{
        destination = "$target"
        index = $routeIndex
        prefix = "$($route.DestinationPrefix)"
        source = $sourceIp
        next_hop = $nextHop
        neighbor = "$neighbor"
    }
}

$internetSettings = Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings'
$result.proxies += ,@('WinINET', 'ProxyEnable', (Text $internetSettings.ProxyEnable))
$result.proxies += ,@('WinINET', 'ProxyServer', (ProxyEndpoints $internetSettings.ProxyServer))
$result.proxies += ,@('WinINET', 'AutoConfigURL host', (ProxyEndpoints $internetSettings.AutoConfigURL))
$result.proxies += ,@('WinINET', 'ProxyOverride configured', "$([bool]$internetSettings.ProxyOverride)")
$result.proxies += ,@('WinINET', 'AutoDetect', (Text $internetSettings.AutoDetect))
$winHttpLines = @(& "$env:SystemRoot\System32\netsh.exe" winhttp show proxy)
$winHttpEndpoints = @($winHttpLines | ForEach-Object {
    if ($_ -match '^(.*?):\s+(.*)$') {
        $value = $Matches[2]
        if ($value -match ':\d+\b') { ProxyEndpoints $value }
    }
})
$result.proxies += ,@('WinHTTP', 'Proxy endpoints',
    $(if ($winHttpEndpoints.Count) { $winHttpEndpoints -join ', ' } else { 'none reported' }))
foreach ($name in @('HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY')) {
    $value = [Environment]::GetEnvironmentVariable($name, 'Process')
    if (!$value) { $value = [Environment]::GetEnvironmentVariable($name, 'User') }
    if ($value) { $result.proxies += ,@('Environment', $name, (ProxyEndpoints $value)) }
}
$driverPattern = '(?i)wintun|windivert|wireguard|tap0901|ovpn|npf|ndiscap|tailscale|zerotier'
foreach ($driver in (Get-CimInstance Win32_SystemDriver | Where-Object {
    $_.Name -match $driverPattern
} | Select-Object -First 32)) {
    $result.interception += ,@('Driver', "$($driver.Name)", "$($driver.State)")
}

$tcpError = @()
$tcp = @(Get-NetTCPConnection -ErrorVariable tcpError -ErrorAction SilentlyContinue)
if ($tcpError.Count) { $result.unavailable += 'TCP sockets' }
$udp = @(Get-NetUDPEndpoint -ErrorAction SilentlyContinue)
$listeners = @($tcp | Where-Object State -eq Listen)
foreach ($socket in ($listeners | Select-Object -First 192)) {
    $owner = Text ($processes[[int]$socket.OwningProcess])
    $result.sockets += ,@('TCP', 'listen', "$($socket.LocalAddress):$($socket.LocalPort)",
        '—', "$($socket.OwningProcess)", $owner)
    if ($owner -match '(?i)mihomo|clash' -and $socket.LocalPort -in @(9090, 9097)) {
        $loopback = switch ($socket.LocalAddress) { '0.0.0.0' { '127.0.0.1' } '::' { '::1' }
            '127.0.0.1' { '127.0.0.1' } '::1' { '::1' } }
        if ($loopback) { $result.controllers += ,@($loopback, "$($socket.LocalPort)", $owner) }
    }
}
foreach ($socket in ($udp | Select-Object -First 96)) {
    $result.sockets += ,@('UDP', 'bound', "$($socket.LocalAddress):$($socket.LocalPort)",
        '—', "$($socket.OwningProcess)", (Text ($processes[[int]$socket.OwningProcess])))
}
foreach ($socket in ($tcp | Where-Object {
    $_.State -eq 'Established' -and (
        $processes[[int]$_.OwningProcess] -match $proxyPattern -or $_.RemoteAddress -in @('127.0.0.1', '::1')
    )
} | Select-Object -First 128)) {
    $owner = Text ($processes[[int]$socket.OwningProcess])
    $local = Format-Endpoint $socket.LocalAddress $socket.LocalPort
    $remote = Format-Endpoint $socket.RemoteAddress $socket.RemotePort
    $result.sockets += ,@('TCP', 'established', $local,
        $remote, "$($socket.OwningProcess)", $owner)
    if ($socket.RemoteAddress -in @('127.0.0.1', '::1')) {
        $peer = $tcp | Where-Object {
            $_.State -eq 'Established' -and $_.LocalAddress -eq $socket.RemoteAddress -and
            $_.LocalPort -eq $socket.RemotePort -and $_.RemoteAddress -eq $socket.LocalAddress -and
            $_.RemotePort -eq $socket.LocalPort
        } | Select-Object -First 1
        if ($peer) {
            $peerName = Text ($processes[[int]$peer.OwningProcess])
            $result.relations += ,@('Local TCP peer', "$owner ($($socket.OwningProcess))", $local,
                "$peerName ($($peer.OwningProcess))", $remote)
        }
    } elseif ($owner -match $proxyPattern) {
        $adapterName = Text ($byAddress[($socket.LocalAddress -split '%')[0]])
        $result.relations += ,@('Proxy upstream socket', "$owner ($($socket.OwningProcess))", $local,
            $adapterName, $remote)
    }
}

$pingTasks = @()
$gatewayTargets = @($routes | ForEach-Object {
    if ($_.NextHop -match '^fe80:' -and $_.NextHop -notmatch '%') { "$($_.NextHop)%$($_.InterfaceIndex)" }
    else { $_.NextHop }
} | Where-Object { $_ -and $_ -notin @('0.0.0.0','::') })
foreach ($target in (($targets + $gatewayTargets) | Select-Object -Unique -First 16)) {
    $ping = [Net.NetworkInformation.Ping]::new()
    try {
        $pingTasks += @{ Address = $target; Ping = $ping; Task = $ping.SendPingAsync("$target", 1100) }
    } catch {
        $ping.Dispose()
        $result.reachability += ,@("$target", 'ICMP', 'unavailable', '—')
    }
}
foreach ($pending in $pingTasks) {
    try {
        $reply = $pending.Task.GetAwaiter().GetResult()
        $result.reachability += ,@("$($pending.Address)", 'ICMP', "$($reply.Status)", "$($reply.RoundtripTime) ms")
    } catch {
        $result.reachability += ,@("$($pending.Address)", 'ICMP', 'unavailable', '—')
    } finally {
        $pending.Ping.Dispose()
    }
}

$result | ConvertTo-Json -Depth 5 -Compress
