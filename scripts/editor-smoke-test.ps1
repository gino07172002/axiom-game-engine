param([int]$Port = 4452)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
. "$PSScriptRoot/env.ps1"
$exe = (Resolve-Path 'target/debug/axiom.exe').Path
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'
$dir = Join-Path $PWD "artifacts/editor-$stamp"
$db = Join-Path $PWD "data/editor-smoke-$stamp.sqlite"
New-Item -ItemType Directory -Force $dir | Out-Null

function Invoke-EditorAxiom([string[]]$Arguments) {
    $raw = & $exe @Arguments
    if ($LASTEXITCODE -ne 0) { throw "Axiom failed: $Arguments" }
    $raw | ConvertFrom-Json
}
function Send-Editor($Request) {
    $client = [Net.Sockets.TcpClient]::new()
    try {
        $client.Connect('127.0.0.1', $Port)
        $client.ReceiveTimeout = 5000
        $stream = $client.GetStream()
        $writer = [IO.StreamWriter]::new($stream, [Text.UTF8Encoding]::new($false), 1024, $true)
        $writer.AutoFlush = $true
        $writer.WriteLine(($Request | ConvertTo-Json -Compress -Depth 30))
        [IO.StreamReader]::new($stream).ReadLine() | ConvertFrom-Json
    } finally { $client.Dispose() }
}
function Wait-Editor([scriptblock]$Predicate) {
    for ($i = 0; $i -lt 150; $i++) {
        if ($process.HasExited) { throw "Editor exited early. See $dir/live-errors.log" }
        try {
            $reply = Send-Editor @{op='observe'}
            if ($reply.ok -and (& $Predicate $reply.observation)) { return $reply.observation }
        } catch {}
        Start-Sleep -Milliseconds 80
    }
    throw 'Editor observation timed out'
}

$baseline = Invoke-EditorAxiom @('--db',$db,'editor','--port','0','--select','stage/crate','--smoke-frames','8','--screenshot',"$dir/before.png")
if ($baseline.observation.editor.selected -ne 'stage/crate' -or $baseline.observation.tick -ne 0) { throw 'Edit mode selection or tick was incorrect' }
$process = Start-Process -FilePath $exe -ArgumentList ('--db "{0}" editor --port {1} --select stage/crate --smoke-frames 360 --screenshot "{2}"' -f $db,$Port,"$dir/after.png") -WorkingDirectory $PWD -WindowStyle Hidden -PassThru -RedirectStandardOutput "$dir/live.log" -RedirectStandardError "$dir/live-errors.log"
try {
    $initial = Wait-Editor { param($o) $o.gpu -and $o.editor.mode -eq 'edit' }
    $discovery = Send-Editor @{op='discover';topic='editor'}
    if ($discovery.schema.read.op -ne 'scene_tree') { throw 'Editor topic is missing' }
    $paged = Send-Editor @{op='scene_tree';query=@{limit=1;max_bytes=256}}
    if (-not $paged.next_cursor -or @($paged.objects).Count -ne 1) { throw 'Scene tree paging failed' }
    $fields = @('/editor','/rev','/activation')
    $first = Send-Editor @{op='observe_delta';fields=$fields}
    $edits = @(
        @{kind='reparent';entity='stage/crate';parent='stage/rack';keep_world=$true},
        @{kind='set_field';target=@{kind='entity';id='stage/crate'};field='/label';value='Signal Crate (AI)'},
        @{kind='set_field';target=@{kind='entity';id='stage/crate'};field='/components/core.transform/position';value=@(3.9,0.64,1.9)}
    )
    $preview = Send-Editor @{op='preview';expected_rev=$initial.rev;edits=$edits}
    if (-not $preview.ok -or '/relations/parent/@stage~1crate' -notin $preview.diff.changes.path) { throw 'Preview did not return a stable hierarchy path' }
    $beforeCommit = Send-Editor @{op='scene_tree';query=@{text='Signal Crate'}}
    if ($beforeCommit.rev -ne $initial.rev -or $beforeCommit.objects[0].parent) { throw 'Preview wrote to the scene' }
    $request = @{op='execute';request_id='scene-edit';request=@{op='apply';expected_rev=$initial.rev;edits=$edits}}
    $applied = Send-Editor $request
    if (-not $applied.ok) { throw ($applied | ConvertTo-Json -Depth 20) }
    $retry = Send-Editor $request
    if (($applied | ConvertTo-Json -Depth 20 -Compress) -ne ($retry | ConvertTo-Json -Depth 20 -Compress)) { throw 'Execute replay changed the authoring receipt' }
    $active = Wait-Editor { param($o) $o.editor.authoring_rev -eq $applied.rev -and $o.rev -eq $applied.rev }
    if ($active.tick -ne 0 -or $active.editor.selected -ne 'stage/crate') { throw 'Live revision changed selection or advanced simulation' }
    $delta = Send-Editor @{op='observe_delta';fields=$fields;cursor=$first.cursor}
    if ($delta.reset -or '/editor/authoring_rev' -notin $delta.changes.path) { throw 'Editor revision was absent from the delta' }
    $stale = Send-Editor @{op='scene_tree';query=@{limit=1;cursor=$paged.next_cursor}}
    if ($stale.diagnostics[0].code -ne 'stale_cursor') { throw 'Scene tree accepted a cursor from a previous revision' }
    $after = Send-Editor @{op='scene_tree';query=@{parent='stage/rack';text='Signal Crate'}}
    if ($after.total -ne 1 -or $after.objects[0].label -ne 'Signal Crate (AI)') { throw 'Live editor graph was not updated' }
    $denied = Send-Editor @{op='save';slot='editor-smoke'}
    if ($denied.ok -or $denied.error -notmatch 'Play preview') { throw 'Editing accepted a simulation mutation' }
    if (-not $process.WaitForExit(90000) -or $process.ExitCode -ne 0) { throw 'Editor did not finish the GPU capture' }
} finally {
    if (-not $process.HasExited) { Stop-Process -Id $process.Id; [void]$process.WaitForExit(5000) }
}
$capture = Get-Content "$dir/after.json" -Raw | ConvertFrom-Json
if ($capture.editor.authoring_rev -ne $applied.rev -or $capture.tick -ne 0) { throw 'GPU capture was not from the committed editor revision' }
$check = Invoke-EditorAxiom @('--db',$db,'verify')
if ($check.rev -ne $applied.rev) { throw 'Committed revision was lost when reopening the database' }
$restored = Invoke-EditorAxiom @('--db',$db,'undo',"$($initial.rev)")
if (-not $restored.ok -or $restored.rev -le $applied.rev) { throw 'Undo did not append a restoration' }
$report = @{
    ok=$true; exe_sha256=(Get-FileHash -LiteralPath $exe).Hash; gpu=$capture.gpu;
    initial_rev=$initial.rev; applied_rev=$applied.rev; restored_rev=$restored.rev;
    changes=$preview.diff.changes; editor_delta=$delta.changes;
    before="$dir/before.png"; after="$dir/after.png"
}
$report | ConvertTo-Json -Depth 30 | Set-Content "$dir/report.json" -Encoding utf8
$report | ConvertTo-Json -Depth 30
