# Live editor <-> ACL parity check on the street demo (docs/acceptance-street.zh-TW.md, part 1).
# Builds the street world twice from its authoring script, applies the same steps once through the
# real GPU editor (editor_view commands = menu / palette commands on the current selection) and
# once through ACL, then compares the exported worlds byte for byte and the revision counts.
# Also captures the editor scene view with the component gizmos. Databases, exports and captures
# live in a new per-run folder under target\street-parity. Refuses a -Port that already answers
# (another editor could receive the writes); -AllowBusyPort skips only that pre-check, the bridge
# must still belong to the editor this script started before the first write.
param([int]$Port = 4462, [string]$Exe = "target\debug\axiom.exe", [switch]$AllowBusyPort)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$Exe = [IO.Path]::GetFullPath([IO.Path]::Combine($root, $Exe))
if (-not (Test-Path -LiteralPath $Exe)) { throw "missing $Exe (build it first: cargo build -p axiom)" }
. (Join-Path $PSScriptRoot 'parity-bridge.ps1')
Assert-PortFree $Port -Allow:$AllowBusyPort
$work = New-RunFolder (Join-Path $root "target\street-parity")
$base = Join-Path $work "base.sqlite"
& powershell -NoProfile -File (Join-Path $PSScriptRoot 'build-street.ps1') -Exe $Exe -Db $base -NoExport | Out-Null
if ($LASTEXITCODE -ne 0) { throw "build-street failed" }
$editorDb = Join-Path $work "editor.sqlite"
$aclDb = Join-Path $work "acl.sqlite"
Copy-Item -LiteralPath $base -Destination $editorDb
Copy-Item -LiteralPath $base -Destination $aclDb

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

$process = Start-Process -FilePath $Exe -ArgumentList ('--db "{0}" editor --port {1}' -f $editorDb, $Port) -WorkingDirectory $root -PassThru -RedirectStandardOutput "$work\editor.log" -RedirectStandardError "$work\editor-errors.log"
$null = $process.Handle
$result = [ordered]@{}
try {
    $first = Wait-Editor { param($o) $o.gpu -and $o.editor.mode -eq 'edit' }
    # The editor that answered must be the one started above (a foreign editor on this port
    # would otherwise receive the first write).
    Assert-BridgeOwner $Port $process.Id
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
    $result.capture = Save-Capture $work 'street-editor-gizmos'
    $null = Send-Editor @{ op = 'editor_view'; select = @('signal'); focus = $true }
    Start-Sleep -Milliseconds 1500
    $result.capture_crossing = Save-Capture $work 'street-editor-crossing'
} finally {
    if (-not $process.HasExited) { Stop-Process -Id $process.Id; [void]$process.WaitForExit(10000) }
}
# Fresh export paths in this run's folder, exit codes checked, compared byte for byte (SHA-256).
$viaEditor = Join-Path $work 'via-editor.json'
$result.editor_sha256 = Export-World $Exe $editorDb $viaEditor
$result.acl_sha256 = Export-World $Exe $aclDb (Join-Path $work 'via-acl.json')
$result.editor_rev = Get-WorldRev $Exe $editorDb
$result.acl_rev = Get-WorldRev $Exe $aclDb
$result.identical = ($result.editor_sha256 -ceq $result.acl_sha256)
$result.bytes = (Get-Item -LiteralPath $viaEditor).Length
$result.work = $work
$result | ConvertTo-Json -Compress
if (-not $result.identical -or $result.editor_rev -ne $result.acl_rev) { exit 1 }
