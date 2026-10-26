param([string]$VisualStudio, [string]$PlatformToolset='v145')
$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$tools=Join-Path $root '.local\tools'
$source=Join-Path $tools 'wix-source'
$commit='b8977d6f88e7b68e000bac226a2814f236770570'
if (-not (Test-Path (Join-Path $source '.git'))) {
    & git clone --branch v7.0.0 --depth 1 https://github.com/wixtoolset/wix.git $source
    if ($LASTEXITCODE) {throw 'WiX source clone failed'}
}
if ((& git -C $source rev-parse HEAD).Trim() -ne $commit) {throw 'Unexpected WiX source revision'}
if (-not $VisualStudio) {
    $vswhere=Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    $VisualStudio=(& $vswhere -latest -prerelease -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath | Select-Object -First 1)
}
$msbuild=Join-Path $VisualStudio 'MSBuild\Current\Bin\amd64\MSBuild.exe'
if (-not (Test-Path $msbuild)) {throw 'Visual Studio C++ build tools and Windows SDK are required'}
$dotnetRoot=Join-Path $tools 'dotnet-9.0.300'
if (-not (Test-Path "$dotnetRoot\dotnet.exe")) {throw 'Run scripts/bootstrap-tools.ps1 -DotnetOnly first'}
$env:DOTNET_ROOT=$dotnetRoot
$env:PATH="$dotnetRoot;$env:PATH"
$env:NUGET_PACKAGES=Join-Path $root '.local\nuget-wix'
$build=Join-Path $source 'build'
New-Item -ItemType Directory -Force "$build\artifacts","$build\native\dutil","$build\native\wixnative" | Out-Null
# Only this isolated checkout is adapted. Source dependencies replace published WiX packages.
# The authoring CLI needs the MSI backend; Burn bundle support is deliberately excluded.
function Original([string]$Relative) { (& git -C $source show "HEAD:$Relative") -join "`n" }
function Put([string]$Relative,[string]$Content) {[IO.File]::WriteAllText((Join-Path $source $Relative),$Content,[Text.UTF8Encoding]::new($false))}
$values=@{packageversion='7.0.0';major='7';minor='0';patch='0';height='0';fullheight='0';nextmajor='8';nextminor='1';label='';dashlabel='';sha=$commit;shortsha=$commit.Substring(0,8);informationalversion='7.0.0';fileversion='7.0.0.0'}
foreach($pair in @(@('Directory.Packages.props.pp','Directory.Packages.props'),@('SomeVerInfo.props.pp','build/SomeVerInfo.props'),@('SomeVerInfo.cs.pp','build/SomeVerInfo.cs'),@('SomeVerInfo.rc.pp','build/SomeVerInfo.rc'))) {
    $text=Original "src/internal/SetBuildNumber/$($pair[0])"
    foreach($key in $values.Keys) {$text=$text.Replace("{$key}",$values[$key])}
    Put $pair[1] $text
}
Put 'global.json' '{"sdk":{"version":"9.0.300","rollForward":"disable"}}'
$projects=@{
 'WixToolset.Data'='src/api/wix/WixToolset.Data/WixToolset.Data.csproj'
 'WixToolset.Extensibility'='src/api/wix/WixToolset.Extensibility/WixToolset.Extensibility.csproj'
 'WixToolset.Versioning'='src/libs/WixToolset.Versioning/WixToolset.Versioning.csproj'
}
$selected=@($projects.Values)+@('src/wix/WixToolset.Converters/WixToolset.Converters.csproj','src/wix/WixToolset.Core/WixToolset.Core.csproj','src/wix/WixToolset.Core.ExtensionCache/WixToolset.Core.ExtensionCache.csproj','src/wix/WixToolset.Core.WindowsInstaller/WixToolset.Core.WindowsInstaller.csproj','src/wix/WixToolset.Core.Native/WixToolset.Core.Native.csproj','src/wix/wix/wix.csproj')
foreach($relative in $selected) {
    $text=Original $relative
    $text=$text.Replace('<TargetFramework>netstandard2.0</TargetFramework>','<TargetFramework>net8.0</TargetFramework>')
    foreach($name in $projects.Keys) {
        $replacement='<ProjectReference Include="'+(Join-Path $source $projects[$name])+'" />'
        $text=$text -replace ('<PackageReference Include="'+[regex]::Escape($name)+'"\s*/>'),$replacement
    }
    $text=$text -replace '(?m)^.*ProjectReference Include="\.\.\\WixToolset.Core.Burn\\.*\r?\n',''
    Put $relative $text
}
$program=Original 'src/wix/wix/Program.cs'
$program=$program.Replace('    using WixToolset.Core.Burn;','').Replace('.AddBundleBackend()','')
Put 'src/wix/wix/Program.cs' $program
# Upstream OSMFEULA.txt section 4 explicitly exempts independently compiled binaries.
# Keep the check in configured official tools; this local derivative uses the MS-RL source grant.
$command=Original 'src/wix/WixToolset.Core/CommandLine/CommandLine.cs'
$command=$command.Replace('command != null && this.RequiresEulaAcceptance(command) && !EulaCommand.IsAccepted(this.AcceptedEulaValues)','false /* independently compiled under MS-RL; OSMFEULA section 4 */')
Put 'src/wix/WixToolset.Core/CommandLine/CommandLine.cs' $command
foreach($relative in @('src/libs/dutil/WixToolset.DUtil/dutil.vcxproj','src/wix/wixnative/wixnative.vcxproj')) {
    $text=Original $relative
    $text=$text -replace '(?m)^.*<PackageReference[^\n]+\r?\n',''
    if($relative -like '*wixnative*') {
        $text=$text.Replace('>crypt32.lib;', '>dutil.lib;crypt32.lib;')
        $settings='<PropertyGroup><ProjectAdditionalIncludeDirectories>'+(Join-Path $source 'src/libs/dutil/WixToolset.DUtil/inc')+'</ProjectAdditionalIncludeDirectories><ProjectAdditionalLinkLibraryDirectories>'+"$build\native\dutil"+'</ProjectAdditionalLinkLibraryDirectories></PropertyGroup>'
        $text=$text.Replace('<ItemGroup Label="ProjectConfigurations">',"$settings`n<ItemGroup Label=`"ProjectConfigurations`">")
    }
    Put $relative $text
}
Push-Location $source
try {
    $common=@('/nologo','/m',"/p:PlatformToolset=$PlatformToolset","/p:WixNativeSdkLibraryToolset=$PlatformToolset",'/p:Configuration=Debug','/p:Platform=x64','/p:TreatWarningAsError=false')
    & $msbuild 'src/libs/dutil/WixToolset.DUtil/dutil.vcxproj' @common "/p:OutDir=$build\native\dutil\"
    if($LASTEXITCODE) {throw 'WiX source DUtil build failed'}
    & $msbuild 'src/wix/wixnative/wixnative.vcxproj' @common "/p:OutDir=$build\native\wixnative\"
    if($LASTEXITCODE) {throw 'WiX source cabinet processor build failed'}
    & "$dotnetRoot\dotnet.exe" build 'src/wix/wix/wix.csproj' -c Debug -p:TargetFrameworks=net8.0 -p:NCrunch=1 -p:UseSharedCompilation=false -o "$tools\wix-msi-source"
    if($LASTEXITCODE) {throw 'WiX source MSI CLI build failed'}
    Copy-Item "$build\native\wixnative\wixnative.exe" "$tools\wix-msi-source\wixnative.exe" -Force
    Copy-Item 'src/wix/wixnative/x64/mergemod.dll' "$tools\wix-msi-source\mergemod.dll" -Force
} finally {Pop-Location}
Write-Output "Built MSI-only WiX 7 authoring CLI from pinned source: $tools\wix-msi-source\wix.dll"
