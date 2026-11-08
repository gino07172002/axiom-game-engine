# Summarize perf-bench JSONL files: per pack, the median over repeats of each metric,
# with the min..max spread across repeats.
#   powershell -File scripts/perf-summary.ps1 artifacts/perf-scratch/bench-release-*.jsonl
param([Parameter(Mandatory, ValueFromRemainingArguments)][string[]]$Files)
$ErrorActionPreference = 'Stop'
$rows = foreach ($f in $Files) { Get-Content -Encoding utf8 $f | Where-Object { $_ } | ForEach-Object { $_ | ConvertFrom-Json } }
$metrics = [ordered]@{
    'step p50 ms'            = { param($r) $r.step.ms.p50 }
    'step p95 ms'            = { param($r) $r.step.ms.p95 }
    'step p99 ms'            = { param($r) $r.step.ms.p99 }
    'step max ms'            = { param($r) $r.step.ms.max }
    'step allocs p50'        = { param($r) $r.step.allocs.p50 }
    'step allocs max'        = { param($r) $r.step.allocs.max }
    'frame ms p50'           = { param($r) $r.render_frame.ms.p50 }
    'frame allocs p50'       = { param($r) $r.render_frame.allocs.p50 }
    'frame KiB p50'          = { param($r) $r.render_frame.alloc_bytes.p50 / 1024 }
    'obs ms p50'             = { param($r) $r.observation.ms.p50 }
    'obs allocs p50'         = { param($r) $r.observation.allocs.p50 }
    'state clone ms p50'     = { param($r) $r.state_clone.ms.p50 }
    'world clone ms p50'     = { param($r) $r.world_clone.ms.p50 }
    'edit->live ms p50'      = { param($r) $r.edit_live.edit_to_live_ms.p50 }
    'edit->live ms p95'      = { param($r) $r.edit_live.edit_to_live_ms.p95 }
    'commit ms p50'          = { param($r) $r.edit_live.commit_ms.p50 }
    'main reload ms p50'     = { param($r) $r.edit_live.main_thread_reload_ms.p50 }
    'main reload ms p95'     = { param($r) $r.edit_live.main_thread_reload_ms.p95 }
    '  read ms p50'          = { param($r) $r.edit_live.read_ms.p50 }
    '  editor clone ms p50'  = { param($r) $r.edit_live.editor_clone_ms.p50 }
    '  editor update ms p50' = { param($r) $r.edit_live.editor_update_ms.p50 }
    '  compile ms p50'       = { param($r) $r.edit_live.compile_ms.p50 }
    '  systems ms p50'       = { param($r) $r.edit_live.systems_ms.p50 }
    '  transfer ms p50'      = { param($r) $r.edit_live.transfer_ms.p50 }
    '  bg request ms p50'     = { param($r) $r.edit_live.request_ms.p50 }
    '  bg adopt ms p50'       = { param($r) $r.edit_live.adopt_ms.p50 }
    '  bg build ms p50'       = { param($r) $r.edit_live.build_ms.p50 }
    'frame ms p95'           = { param($r) $r.render_frame.ms.p95 }
    'working set MiB'        = { param($r) $r.process_memory_after_runtime.working_set_bytes / 1MB }
}
function Med([double[]]$v) { $s = @($v | Sort-Object); $s[[math]::Floor(($s.Count - 1) / 2)] }
$packs = $rows | Select-Object -ExpandProperty pack -Unique
foreach ($pack in $packs) {
    $set = @($rows | Where-Object { $_.pack -eq $pack })
    "== $pack ($($set[0].build_profile), repeats $($set.Count), entities $($set[0].entities), instances $($set[0].instances))"
    foreach ($name in $metrics.Keys) {
        $values = @($set | ForEach-Object { [double](& $metrics[$name] $_) })
        $sorted = @($values | Sort-Object)
        '{0,-24} {1,10:N3}   [{2:N3} .. {3:N3}]' -f $name, (Med $values), $sorted[0], $sorted[-1]
    }
}
