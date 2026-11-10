# Rasterizes assets/editor-icons/*.svg into the editor's icon atlas (atlas.png + atlas.json).
# Build-time only: the editor embeds the committed PNG and never parses SVG at runtime.
# Uses the installed Microsoft Edge (headless) as the rasterizer; rerun after editing an icon.
param(
    [string]$Edge = 'C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe'
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$dir = Join-Path $root 'assets\editor-icons'
$utf8 = New-Object Text.UTF8Encoding $false
$manifest = [IO.File]::ReadAllText((Join-Path $dir 'icons.json'), $utf8) | ConvertFrom-Json
$names = @($manifest.icons | ForEach-Object { $_.name })
if ($names.Count -eq 0) { throw 'icons.json lists no icons' }

# Two levels in one image: 40 px cells (large / HiDPI) on top, 20 px cells below. The editor
# picks the level whose cell is closest to (and not smaller than) the drawn pixel size.
$columns = 16
$rows = [Math]::Ceiling($names.Count / $columns)
$levels = @(@{cell = 40; y = 0 }, @{cell = 20; y = 40 * $rows })
$width = $columns * 40
$height = 40 * $rows + 20 * $rows

$html = New-Object Text.StringBuilder
[void]$html.Append('<!doctype html><html><head><meta charset="utf-8"><style>html,body{margin:0;padding:0;background:transparent;overflow:hidden}svg{position:absolute;display:block;color:#fff}</style></head><body>')
for ($i = 0; $i -lt $names.Count; $i++) {
    $file = Join-Path $dir ($manifest.icons[$i].file)
    $svg = [IO.File]::ReadAllText($file, $utf8).Trim()
    foreach ($level in $levels) {
        $c = $level.cell
        $x = ($i % $columns) * $c
        $y = $level.y + [Math]::Floor($i / $columns) * $c
        $sized = $svg -replace 'width="24" height="24"', ('width="{0}" height="{0}" style="left:{1}px;top:{2}px"' -f $c, $x, $y)
        if ($sized -eq $svg) { throw "$($manifest.icons[$i].file): expected width=""24"" height=""24""" }
        [void]$html.Append($sized)
    }
}
[void]$html.Append('</body></html>')

$work = Join-Path ([IO.Path]::GetTempPath()) ('axiom-icons-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $work | Out-Null
try {
    $page = Join-Path $work 'atlas.html'
    [IO.File]::WriteAllText($page, $html.ToString(), $utf8)
    $png = Join-Path $work 'atlas.png'
    $edgeArgs = @('--headless', '--disable-gpu', '--hide-scrollbars', '--force-device-scale-factor=1',
        '--default-background-color=00000000', "--user-data-dir=$work\profile",
        "--window-size=$width,$height", "--screenshot=$png", ('file:///' + ($page -replace '\\', '/')))
    $p = Start-Process -FilePath $Edge -ArgumentList $edgeArgs -Wait -PassThru -WindowStyle Hidden
    if (-not (Test-Path $png)) { throw "Edge did not write the atlas (exit $($p.ExitCode))" }
    $bytes = [IO.File]::ReadAllBytes($png)
    $w = [BitConverter]::ToUInt32(@($bytes[19], $bytes[18], $bytes[17], $bytes[16]), 0)
    $h = [BitConverter]::ToUInt32(@($bytes[23], $bytes[22], $bytes[21], $bytes[20]), 0)
    if ($w -ne $width -or $h -ne $height -or $bytes[25] -ne 6) { throw "unexpected atlas $w x $h color type $($bytes[25])" }
    Copy-Item $png (Join-Path $dir 'atlas.png') -Force
    $meta = [ordered]@{
        version = 1; columns = $columns; width = $width; height = $height
        levels = @($levels | ForEach-Object { [ordered]@{ cell = $_.cell; y = $_.y } })
        names = $names
        source = 'scripts/build-editor-icons.ps1 (Edge headless, white on transparent)'
    }
    [IO.File]::WriteAllText((Join-Path $dir 'atlas.json'), ($meta | ConvertTo-Json -Depth 5), $utf8)
    [ordered]@{ ok = $true; icons = $names.Count; width = $width; height = $height; bytes = (Get-Item (Join-Path $dir 'atlas.png')).Length } | ConvertTo-Json -Compress
} finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
