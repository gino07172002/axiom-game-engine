# Live editor <-> ACL parity check on the street demo (docs/acceptance-street.zh-TW.md, part 1).
# Builds the street world twice from its authoring script, applies the same steps once through the
# real GPU editor (editor_view commands = menu / palette commands on the current selection) and
# once through ACL, then compares the exported worlds byte for byte and the revision counts.
# Also captures the editor scene view with the component gizmos. Databases live under target/.
param([int]$Port = 4462, [string]$Exe = "target\debug\axiom.exe")
$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)
$work = Join-Path $PWD "target\street-parity"
New-Item -ItemType Directory -Force $work | Out-Null
$base = Join-Path $work "base.sqlite"
& powershell -NoProfile -File scripts\build-street.ps1 -Exe $Exe -Db $base -NoExport | Out-Null
if ($LASTEXITCODE -ne 0) { throw "build-street failed" }
$editorDb = Join-Path $work "editor.sqlite"
$aclDb = Join-Path $work "acl.sqlite"
Remove-Item "$editorDb*", "$aclDb*" -ErrorAction SilentlyContinue
Copy-Item $base $editorDb
Copy-Item $base $aclDb

function Send-Editor($Request) {
    $client = [Net.Sockets.TcpClient]::new()
    try {
        $client.Connect('127.0.0.1', $Port)
        $client.ReceiveTimeout = 30000
        $stream = $client.GetStream()
        $writer = [IO.StreamWriter]::new($stream, [Text.UTF8Encoding]::new($false), 1024, $true)
        $writer.AutoFlush = $true
        $writer.WriteLine(($Request | ConvertTo-Json -Compress -Depth 30))
        [IO.StreamReader]::new($stream, [Text.UTF8Encoding]::new($false)).ReadLine() | ConvertFrom-Json
    } finally { $client.Dispose() }
}
function Wait-Editor([scriptblock]$Predicate) {
    for ($i = 0; $i -lt 400; $i++) {
        if ($process.HasExited) { throw "Editor exited early. See $work\editor-errors.log" }
        try {
            $reply = Send-Editor @{op = 'observe' }
            if ($reply.ok -and (& $Predicate $reply.observation)) { return $reply.observation }
        } catch {}
        Start-Sleep -Milliseconds 100
    }
    throw 'Editor observation timed out'
}

# Same steps, two front ends. Each step is one revision on both sides.
$steps = @(
    @{ select = @();                          command = 'object.cube';          acl = 'create cube' },
    @{ select = @('cube');                    command = 'object.interactable';  acl = 'interact cube' },
    @{ select = @('cube');                    command = 'object.audio_source';  acl = 'audio cube source' },
    @{ select = @('cube');                    command = 'object.tween_clip';    acl = 'tween cube clip clip' },
    @{ select = @();                          command = 'object.obstacle';      acl = 'create obstacle' },
    @{ select = @();                          command = 'object.trigger_box';   acl = 'create trigger box' },
    @{ select = @('camera', 'ped/n1');        command = 'object.camera_follow'; acl = 'camera camera follow ped/n1' },
    @{ select = @('ped/s1');                  command = 'edit.duplicate';       acl = 'duplicate ped/s1' }
)

$process = Start-Process -FilePath $Exe -ArgumentList ('--db "{0}" editor --port {1}' -f $editorDb, $Port) -WorkingDirectory $PWD -PassThru -RedirectStandardOutput "$work\editor.log" -RedirectStandardError "$work\editor-errors.log"
$null = $process.Handle
$result = [ordered]@{}
try {
    $first = Wait-Editor { param($o) $o.gpu -and $o.editor.mode -eq 'edit' }
    $rev = $first.editor.authoring_rev
    $result.start_rev = $rev
    $editorMs = @()
    foreach ($step in $steps) {
        $request = @{ op = 'editor_view'; command = $step.command }
        if ($step.select.Count -gt 0) { $request.select = $step.select }
        $t = [Diagnostics.Stopwatch]::StartNew()
        $reply = Send-Editor $request
        if (-not $reply.ok) { throw "editor $($step.command): $($reply | ConvertTo-Json -Depth 10 -Compress)" }
        $expected = $rev + 1
        $null = Wait-Editor { param($o) $o.editor.authoring_rev -eq $expected -and $o.rev -eq $expected }
        $editorMs += [math]::Round($t.Elapsed.TotalMilliseconds)
        $rev = $expected
        $out = & $Exe --db $aclDb acl -e $step.acl 2>&1 | Out-String
        if ($LASTEXITCODE -ne 0) { throw "acl $($step.acl): $out" }
    }
    $result.editor_step_ms = $editorMs
    # Gizmo screenshot: frame the cafe door with paths, triggers, nav, audio and camera gizmos visible.
    $null = Send-Editor @{ op = 'editor_view'; select = @('cafe/door'); focus = $true }
    Start-Sleep -Milliseconds 1500
    $shot = Send-Editor @{ op = 'editor_view'; capture = 'street-editor-gizmos' }
    $result.capture = $shot.capture
    Start-Sleep -Milliseconds 2500
    $null = Send-Editor @{ op = 'editor_view'; select = @('signal'); focus = $true }
    Start-Sleep -Milliseconds 1500
    $shot = Send-Editor @{ op = 'editor_view'; capture = 'street-editor-crossing' }
    $result.capture_crossing = $shot.capture
    Start-Sleep -Milliseconds 2500
} finally {
    if (-not $process.HasExited) { Stop-Process -Id $process.Id; [void]$process.WaitForExit(10000) }
}
& $Exe --db $editorDb export "$work\via-editor.json" | Out-Null
& $Exe --db $aclDb export "$work\via-acl.json" | Out-Null
$a = [IO.File]::ReadAllText("$work\via-editor.json")
$b = [IO.File]::ReadAllText("$work\via-acl.json")
$result.editor_rev = (& $Exe --db $editorDb query | ConvertFrom-Json).rev
$result.acl_rev = (& $Exe --db $aclDb query | ConvertFrom-Json).rev
$result.identical = ($a -eq $b)
$result.bytes = $a.Length
$result | ConvertTo-Json -Compress
if (-not $result.identical -or $result.editor_rev -ne $result.acl_rev) { exit 1 }
