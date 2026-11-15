param([Parameter(Mandatory)][string]$Directory)
$ErrorActionPreference='Stop'
$directoryPath=(Resolve-Path -LiteralPath $Directory).Path
$installer=New-Object -ComObject WindowsInstaller.Installer
$client=$installer.OpenDatabase((Join-Path $directoryPath 'Client.msi'),0)
$service=$installer.OpenDatabase((Join-Path $directoryPath 'Service.msi'),0)
function Property($Database,[string]$Name) {
    $view=$Database.OpenView("SELECT ``Value`` FROM ``Property`` WHERE ``Property`` = '$Name'")
    $view.Execute(); $record=$view.Fetch()
    if($record) {return [string]$record.StringData(1)}
    return ''
}
if((Property $client 'UpgradeCode') -ne '{81E2553E-095D-49AF-A95A-B0FD47A3E61E}' -or (Property $service 'UpgradeCode') -ne '{7FBA8135-7341-4F91-A39E-615190032EA2}') {throw 'MSI UpgradeCode mismatch'}
if((Property $client 'ALLUSERS') -ne '' -or (Property $service 'ALLUSERS') -ne '1') {throw 'Client/service installation scope mismatch'}
if((Property $client 'ProductCode') -eq (Property $service 'ProductCode')) {throw 'Client/service ProductCode must be independent'}
$view=$service.OpenView('SELECT `Name`,`Arguments`,`StartName` FROM `ServiceInstall`'); $view.Execute(); $record=$view.Fetch()
if(-not $record -or $record.StringData(1) -ne 'BitNet' -or $record.StringData(2) -ne '--service' -or $record.StringData(3) -ne 'NT SERVICE\BitNet') {throw 'SCM service identity/arguments mismatch'}
$view=$service.OpenView('SELECT `Action` FROM `InstallExecuteSequence` WHERE `Action` = ''MsiConfigureServices''')
$view.Execute()
if($view.Fetch()) {throw 'Service MSI must leave recovery configuration to bitmaint after installation'}
$view=$service.OpenView('SELECT `SDDLText` FROM `MsiLockPermissionsEx`'); $view.Execute(); $record=$view.Fetch()
if(-not $record) {throw 'Protected service-data ACL is absent'}
$sddl=[string]$record.StringData(1)
if(-not $sddl.StartsWith('D:P') -or -not $sddl.Contains('S-1-5-80-') -or $sddl.Contains(';;;WD)') -or $sddl.Contains(';;;AU)') -or $sddl.Contains(';;;BU)')) {throw 'Unsafe service-data DACL'}
$componentGuids=@{}
foreach($database in @($client,$service)) {
    $view=$database.OpenView('SELECT `ComponentId` FROM `Component`'); $view.Execute()
    while($record=$view.Fetch()) {
        $guid=[string]$record.StringData(1)
        if($componentGuids.ContainsKey($guid)) {throw 'Client/service components unexpectedly share a GUID'}
        $componentGuids[$guid]=$true
    }
}
$view=$client.OpenView('SELECT `PropVariantValue` FROM `MsiShortcutProperty` WHERE `PropertyKey` = ''System.AppUserModel.ID'''); $view.Execute(); $record=$view.Fetch()
if(-not $record -or $record.StringData(1) -ne 'top.nsapi.bitnet.gui') {throw 'Start-menu notification AUMID is absent'}
$view=$client.OpenView('SELECT `Condition` FROM `InstallExecuteSequence` WHERE `Action` = ''RemoveUserStartup'''); $view.Execute(); $record=$view.Fetch()
if(-not $record -or $record.StringData(1) -ne 'REMOVE = "ALL" AND NOT UPGRADINGPRODUCTCODE') {throw 'Startup cleanup must run only on explicit removal, never a major upgrade'}
foreach($database in @($client,$service)) {
    $view=$database.OpenView('SELECT `Condition` FROM `LaunchCondition`'); $view.Execute(); $guarded=$false
    while($record=$view.Fetch()) {if($record.StringData(1) -eq 'NOT BITNET_NEWER_PRODUCT_FOUND OR BITNET_ALLOW_DOWNGRADE = "1"') {$guarded=$true}}
    if(-not $guarded) {throw 'Explicit rollback-only downgrade guard is absent'}
}
Write-Output 'Read-only MSI audit passed: independent scope/products/components, fixed virtual service identity, protected data DACL and own notification identity. No installation was performed.'
