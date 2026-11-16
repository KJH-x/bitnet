param([string]$Version='0.1.2',[switch]$WorkingTree)
$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
New-Item -ItemType Directory -Force "$root\dist\source" | Out-Null
if($WorkingTree) {
    New-Item -ItemType Directory -Force "$root\.local" | Out-Null
    $listing=Join-Path $root ".local\source-files-$([Guid]::NewGuid().ToString('N')).txt"
    $files=@(& git -C $root -c core.quotepath=false ls-files --cached --others --exclude-standard)
    if($LASTEXITCODE) {throw 'Cannot enumerate source files'}
    foreach($relative in $files) {
        $resolved=[IO.Path]::GetFullPath((Join-Path $root $relative))
        if(-not $resolved.StartsWith($root+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase) -or $relative.Contains("`n") -or $relative.Contains("`r") -or (Get-Item -LiteralPath $resolved).Attributes.HasFlag([IO.FileAttributes]::ReparsePoint)) {throw 'Unsafe path in source listing'}
    }
    [IO.File]::WriteAllLines($listing,$files,[Text.UTF8Encoding]::new($false))
    & tar -czf "$root\dist\source\bitnet-$Version-source-working-tree.tar.gz" -C $root -T $listing
    if($LASTEXITCODE) {throw 'Working-tree source archive failed'}
} else {
    & git -C $root archive --format=tar.gz "--prefix=bitnet-$Version/" "--output=$root\dist\source\bitnet-$Version-source.tar.gz" HEAD
    if($LASTEXITCODE) {throw 'Source archive requires a committed source tree; use -WorkingTree for an explicit development snapshot'}
}
