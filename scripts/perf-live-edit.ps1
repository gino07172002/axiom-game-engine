# Live edit -> live latency in the real editor (window hidden, GPU renderer active).
# Sends N one-field `set_field` applies through the bridge, waits until `observe` reports
# the new revision as applied, and records `metrics.reload` (main-thread phases, including
# GPU shader/pipeline preparation). Uses an isolated database under artifacts/perf-scratch.
#   powershell -File scripts/perf-live-edit.ps1 -Pack content/effects-tour -Entity station/01 -Position 1,2,3
param(
    [Parameter(Mandatory)][string]$Pack,
    [string]$Entity = '',
    # Comma-separated x,y,z (string so `-File` invocations parse it).
    [string]$Position = '0,0,0',
    [int]$Edits = 12,
    # Shader mode: edit `render.shaders` program WGSL instead of a position. Edits alternate
    # between the file text and the text plus a comment (like undo/redo of a shader edit).
    [string]$ShaderProgram = '',
    [string]$ShaderFile = '',
    # Editor mode: each edit is an editor commit (`editor_view` running this registered command,
    # e.g. object.cube), committed by the editor itself instead of the bridge's own store.
    # The reply arrives once the commit landed; `bridge_commit_ms` is then that reply time.
    [string]$EditorCommand = '',
    # Optional: select "<prefix><i>" for edit i in the same request (e.g. -EditorCommand edit.delete
    # -EditorSelectPrefix box/ on a world at its entity budget).
    [string]$EditorSelectPrefix = '',
    [int]$Port = 4471,
    [string]$Exe = 'target/release/axiom.exe',
    [string]$Out = ''
)
$ErrorActionPreference = 'Stop'
$pos = [double[]]($Position.Split(","))
$shaderText = if ($ShaderFile) { [IO.File]::ReadAllText((Resolve-Path $ShaderFile).Path) } else { '' }
Set-Location (Join-Path $PSScriptRoot '..')
. "$PSScriptRoot/env.ps1"
$exePath = (Resolve-Path $Exe).Path
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'
$dir = Join-Path $PWD "artifacts/perf-scratch/live-$stamp"
New-Item -ItemType Directory -Force $dir | Out-Null
$db = Join-Path $dir 'live.sqlite'

function Send-Bridge($Request) {
    $client = [Net.Sockets.TcpClient]::new()
    try {
        $client.Connect('127.0.0.1', $Port)
        $client.ReceiveTimeout = 10000
        $stream = $client.GetStream()
        $writer = [IO.StreamWriter]::new($stream, [Text.UTF8Encoding]::new($false), 1024, $true)
        $writer.AutoFlush = $true
        $writer.WriteLine(($Request | ConvertTo-Json -Compress -Depth 30))
        [IO.StreamReader]::new($stream).ReadLine() | ConvertFrom-Json
    } finally { $client.Dispose() }
}
function Wait-Observation([scriptblock]$Predicate, [int]$TimeoutMs = 60000) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while ($watch.ElapsedMilliseconds -lt $TimeoutMs) {
        if ($process.HasExited) { throw "Editor exited early. See $dir/errors.log" }
        try {
            $reply = Send-Bridge @{op = 'observe' }
            if ($reply.ok -and (& $Predicate $reply.observation)) { return $reply.observation }
        } catch {}
        Start-Sleep -Milliseconds 5
    }
    throw 'observation timed out'
}

$process = Start-Process -FilePath $exePath -ArgumentList ('--db "{0}" --pack "{1}" editor --port {2} --smoke-frames 1000000' -f $db, $Pack, $Port) -WorkingDirectory $PWD -WindowStyle Hidden -PassThru -RedirectStandardOutput "$dir/out.log" -RedirectStandardError "$dir/errors.log"
$null = $process.Handle
$rows = @()
try {
    $obs = Wait-Observation { param($o) $o.gpu -and $o.editor.mode -eq 'edit' }
    $rev = [int64]$obs.rev
    for ($i = 0; $i -le $Edits; $i++) {
        $p = @(($pos[0] + 0.001 * ($i + 1)), $pos[1], $pos[2])
        $watch = [Diagnostics.Stopwatch]::StartNew()
        if ($EditorCommand) {
            $view = @{op = 'editor_view'; command = $EditorCommand }
            if ($EditorSelectPrefix) { $view.select = @("$EditorSelectPrefix$i") }
            $reply = Send-Bridge $view
            if (-not $reply.ok) { throw "editor command failed: $($reply | ConvertTo-Json -Compress -Depth 10)" }
            $commitMs = $watch.Elapsed.TotalMilliseconds
            $next = $rev + 1
            $live = Wait-Observation { param($o) [int64]$o.rev -eq $next -and $o.activation -eq 'applied' -and $o.editor.authoring_rev -eq $next }
            $rev = $next
            if ($i -eq 0) { continue }
            $last = $live.reload.last
            $rows += [pscustomobject]@{
                client_edit_to_live_ms = $watch.Elapsed.TotalMilliseconds
                bridge_commit_ms       = $commitMs
                main_thread_ms         = $last.total_ms
                writer_commit_ms       = [double]$live.editor.last_commit_ms
                slow_frames            = [double]$live.frame.slow_frames
            }
            continue
        }
        $edit = if ($ShaderProgram) {
            $text = if ($i % 2 -eq 0) { $shaderText + "`n// perf edit" } else { $shaderText }
            @{kind = 'set_field'; target = @{kind = 'resource'; id = 'render.shaders' }; field = "/shaders/$ShaderProgram/wgsl"; value = $text }
        } else {
            @{kind = 'set_field'; target = @{kind = 'entity'; id = $Entity }; field = '/components/core.transform/position'; value = $p }
        }
        $reply = Send-Bridge @{op = 'apply'; expected_rev = $rev; edits = @($edit) }
        if (-not $reply.ok) { throw "apply failed: $($reply | ConvertTo-Json -Compress -Depth 10)" }
        $commitMs = $watch.Elapsed.TotalMilliseconds
        $next = [int64]$reply.rev
        $live = Wait-Observation { param($o) [int64]$o.rev -eq $next -and $o.activation -eq 'applied' -and $o.reload.last.rev -eq $next }
        $rev = $next
        if ($i -eq 0) { continue } # warm-up
        $last = $live.reload.last
        $rows += [pscustomobject]@{
            client_edit_to_live_ms = $watch.Elapsed.TotalMilliseconds
            bridge_commit_ms       = $commitMs
            main_thread_ms         = $last.total_ms
            read_ms                = $last.read_ms
            editor_clone_ms        = $last.editor_clone_ms
            editor_update_ms       = $last.editor_update_ms
            compile_ms             = $last.reconcile.build.compile_ms
            systems_ms             = $last.reconcile.build.systems_ms
            transfer_ms            = $last.reconcile.transfer_ms
            prepare_gpu_ms         = $last.reconcile.prepare_ms
            reconcile_ms           = $last.reconcile.total_ms
        }
    }
    $final = Send-Bridge @{op = 'observe' }
    $history = Send-Bridge @{op = 'history'; limit = 64 }
} finally {
    if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
}
function Stat($name) {
    $v = @($rows | ForEach-Object { [double]$_.$name } | Sort-Object)
    $n = $v.Count
    [pscustomobject]@{ metric = $name; p50 = $v[[math]::Ceiling(($n - 1) * 0.5)]; p95 = $v[[math]::Ceiling(($n - 1) * 0.95)]; min = $v[0]; max = $v[$n - 1] }
}
$names = if ($EditorCommand) { @('client_edit_to_live_ms', 'bridge_commit_ms', 'main_thread_ms', 'writer_commit_ms') } else { @('client_edit_to_live_ms', 'bridge_commit_ms', 'main_thread_ms', 'read_ms', 'editor_clone_ms', 'editor_update_ms', 'compile_ms', 'systems_ms', 'transfer_ms', 'prepare_gpu_ms', 'reconcile_ms') }
$summary = $names | ForEach-Object { Stat $_ }
$result = [pscustomobject]@{ pack = $Pack; exe = $Exe; editor_command = $EditorCommand; edits = $rows.Count; summary = $summary; samples = $rows; reload_counters = $final.observation.reload; frame = $final.observation.frame; editor_writes = $final.observation.editor.writes; history = $history.revisions }
$json = $result | ConvertTo-Json -Depth 8
if ($Out) { [IO.File]::WriteAllText((Join-Path $PWD $Out), $json) }
$summary | Format-Table -AutoSize | Out-String | Write-Output
