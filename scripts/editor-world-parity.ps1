# Live editor <-> ACL parity from an EMPTY world (GAP-1 / GAP-3, docs/acceptance-street.zh-TW.md).
# Two fresh databases from content/empty. One is driven only through the real GPU editor's
# commands (editor_view {select, command, arg} = the World window / Inspector tag chips / menus),
# the other only through ACL; the exported worlds must be byte-identical with the same revision.
# Captures the World window and the Inspector tag chips. Databases, exports and captures live in a
# new per-run folder under target\world-parity. Refuses a -Port that already answers (another
# editor could receive the writes); -AllowBusyPort skips only that pre-check, the bridge must
# still belong to the editor this script started before the first write.
param([int]$Port = 4466, [string]$Exe = "target\debug\axiom.exe", [switch]$AllowBusyPort)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$Exe = [IO.Path]::GetFullPath([IO.Path]::Combine($root, $Exe))
if (-not (Test-Path -LiteralPath $Exe)) { throw "missing $Exe (build it first: cargo build -p axiom)" }
. (Join-Path $PSScriptRoot 'parity-bridge.ps1')
Assert-PortFree $Port -Allow:$AllowBusyPort
$work = New-RunFolder (Join-Path $root "target\world-parity")
$editorDb = Join-Path $work "editor.sqlite"
$aclDb = Join-Path $work "acl.sqlite"
foreach ($db in $editorDb, $aclDb) {
    & $Exe --db $db --pack content\empty query | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "seed $db failed" }
}

function Wait-Editor([scriptblock]$Predicate) {
    for ($i = 0; $i -lt 600; $i++) {
        if ($process.HasExited) { throw "Editor exited early. See $work\editor-errors.log" }
        try {
            $reply = Send-Editor @{op = 'observe' }
            if ($reply.ok -and (& $Predicate $reply.observation)) { return $reply.observation }
        } catch {}
        Start-Sleep -Milliseconds 100
    }
    throw 'Editor observation timed out'
}

# Each step is one revision on both sides. Editor: World window / tag chips / menus via their
# registered commands (with the argument a person picks in the UI). ACL: the same in text.
# CJK title from code points (Windows PowerShell 5.1 reads BOM-less scripts as ANSI).
$title = [regex]::Unescape('\u7de8\u8f2f\u5668\u8857\u53e3 Corner')
$steps = @(
    @{ select = @();          command = 'world.set_id';          arg = 'corner';              acl = 'world id corner' },
    @{ select = @();          command = 'world.set_title';       arg = $title;               acl = "world title `"$title`"" },
    @{ select = @();          command = 'world.capability_add';  arg = 'physics';             acl = 'capability add physics' },
    @{ select = @();          command = 'world.resource_create'; arg = 'behavior.program';    acl = 'resource create behavior.program' },
    @{ select = @();          command = 'world.resource_create'; arg = 'nav.grid';            acl = 'resource create nav.grid' },
    @{ select = @();          command = 'world.system_add';      arg = 'trigger.detect';      acl = 'system add trigger.detect' },
    @{ select = @();          command = 'object.cube';           arg = $null;                 acl = 'create cube' },
    @{ select = @('cube');    command = 'object.tag';            arg = '+player';             acl = 'tag cube +player' },
    @{ select = @();          command = 'object.cube';           arg = $null;                 acl = 'create cube' },
    @{ select = @('cube/2');  command = 'object.interactable';   arg = $null;                 acl = 'interact cube/2' },
    @{ select = @('cube/2');  command = 'object.add_component';  arg = 'physics.collider';    acl = 'add cube/2 physics.collider' },
    @{ select = @();          command = 'object.trigger_box';    arg = $null;                 acl = 'create trigger box' }
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
    $n = 0
    foreach ($step in $steps) {
        $n++
        $request = @{ op = 'editor_view'; command = $step.command }
        if ($step.select.Count -gt 0) { $request.select = $step.select }
        if ($null -ne $step.arg) { $request.arg = $step.arg }
        $t = [Diagnostics.Stopwatch]::StartNew()
        $reply = Send-Editor $request
        if (-not $reply.ok) { throw "editor $($step.command): $($reply | ConvertTo-Json -Depth 10 -Compress)" }
        $expected = $rev + 1
        $null = Wait-Editor { param($o) $o.editor.authoring_rev -eq $expected -and $o.rev -eq $expected }
        $editorMs += [math]::Round($t.Elapsed.TotalMilliseconds)
        $rev = $expected
        # ACL scripts go through a UTF-8 file: native args lose embedded quotes on PowerShell 5.1.
        $file = Join-Path $work ("step-{0:d2}.acl" -f $n)
        [IO.File]::WriteAllText($file, $step.acl, [Text.UTF8Encoding]::new($false))
        $out = & $Exe --db $aclDb acl $file 2>&1 | Out-String
        if ($LASTEXITCODE -ne 0) { throw "acl $($step.acl): $out" }
    }
    $result.editor_step_ms = $editorMs
    # Screenshot: the World window open, the tagged cube selected (Inspector tag chips).
    $reply = Send-Editor @{ op = 'editor_view'; command = 'window.world'; select = @('cube'); focus = $true }
    if (-not $reply.ok) { throw "window.world: $($reply | ConvertTo-Json -Depth 10 -Compress)" }
    Start-Sleep -Milliseconds 2500
    $result.capture = Save-Capture $work 'editor-world-panel'
    # Screenshot: a resource from the World window opened in the Inspector (schema form).
    $reply = Send-Editor @{ op = 'editor_view'; open = 'resource:nav.grid' }
    if (-not $reply.ok) { throw "open resource: $($reply | ConvertTo-Json -Depth 10 -Compress)" }
    Start-Sleep -Milliseconds 2500
    $result.capture_resource = Save-Capture $work 'editor-resource-form'
    $result.final_observation = (Send-Editor @{ op = 'observe' }).observation.editor | Select-Object authoring_rev, selection, pending_commits
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
$result | ConvertTo-Json -Compress -Depth 6
if (-not $result.identical -or $result.editor_rev -ne $result.acl_rev) { exit 1 }
