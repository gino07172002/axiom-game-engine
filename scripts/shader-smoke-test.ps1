param([int]$Port = 4422, [switch]$SkipEarlier)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
. "$PSScriptRoot/env.ps1"
$exe = (Resolve-Path 'target/debug/axiom.exe').Path
$hash = (Get-FileHash -LiteralPath $exe).Hash
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'
$reportDir = Join-Path $PWD "artifacts/shader-$stamp"
$db = Join-Path $PWD "data/shader-$stamp.sqlite"
New-Item -ItemType Directory -Force $reportDir | Out-Null

function Invoke-ShaderAxiom([string[]]$Arguments) {
    $output = & $exe @Arguments
    if ($LASTEXITCODE -ne 0) { throw "Axiom failed: $Arguments" }
    $output | ConvertFrom-Json
}
function Send-Shader($Request) {
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
function Wait-Shader([scriptblock]$Predicate) {
    for ($attempt = 0; $attempt -lt 150; $attempt++) {
        if ($process.HasExited) { throw "Shader preview exited early. See $reportDir/live-errors.log" }
        try {
            $reply = Send-Shader @{op='observe'}
            if ($reply.ok -and (& $Predicate $reply.observation)) { return $reply.observation }
        } catch {}
        Start-Sleep -Milliseconds 80
    }
    throw 'Shader activation timed out'
}
function Set-ShaderSource([string]$Op, [long]$Rev, [string]$Source) {
    Send-Shader @{op=$Op;expected_rev=$Rev;edits=@(@{kind='set_field';target=@{kind='resource';id='render.shaders'};field='/shaders/scan/wgsl';value=$Source})}
}

if (-not $SkipEarlier) { & "$PSScriptRoot/m15-smoke-test.ps1" -Port ($Port + 1) }
$baseline = Invoke-ShaderAxiom @('--db',$db,'--pack','content/shader-lab','play','--start','--port','0','--smoke-frames','12','--screenshot',"$reportDir/before.png")
if ($baseline.observation.shader.materials -ne 6) { throw 'Shader lab did not activate' }
$process = Start-Process -FilePath $exe -ArgumentList ('--db "{0}" play --start --port {1} --smoke-frames 480 --screenshot "{2}"' -f $db,$Port,"$reportDir/after.png") -WindowStyle Hidden -PassThru -RedirectStandardOutput "$reportDir/live.log" -RedirectStandardError "$reportDir/live-errors.log"
try {
    $initial = Wait-Shader { param($o) $o.gpu -and $o.shader.materials -eq 6 }
    $invalid = Set-ShaderSource preview $initial.rev "fn surface(s: SurfaceInput) -> vec4<f32> {`nreturn not_a_variable;`n}"
    if ($invalid.ok -or $invalid.diagnostics[0].code -ne 'invalid_shader' -or $invalid.diagnostics[0].path -ne '/resources/render.shaders/shaders/scan/wgsl') { throw 'Invalid WGSL did not yield a scoped diagnostic' }
    $red = 'fn surface(s: SurfaceInput) -> vec4<f32> { return vec4<f32>(0.95, 0.04, 0.02, 1.0); }'
    $applied = Set-ShaderSource apply $initial.rev $red
    if (-not $applied.ok) { throw ($applied | ConvertTo-Json -Depth 20) }
    $active = Wait-Shader { param($o) $o.rev -eq $applied.rev -and $o.activation -eq 'applied' }
    # This function compiles alone, then collides with the GPU wrapper's vertex entry point.
    $failure = Set-ShaderSource apply $active.rev ($red + "`nfn vs_main() {}")
    if (-not $failure.ok) { throw 'Expected the isolated hook to pass CPU validation' }
    $retained = Wait-Shader { param($o) $o.requested_rev -eq $failure.rev -and $o.activation -eq 'activation_failed' }
    if ($retained.rev -ne $active.rev -or $retained.activation_error.diagnostics[0].code -ne 'shader_pipeline') { throw 'Failed pipeline replaced the active revision' }
    if (-not $process.WaitForExit(60000) -or $process.ExitCode -ne 0) { throw 'Shader preview stopped before capture' }
} finally {
    if (-not $process.HasExited) { Stop-Process -Id $process.Id; [void]$process.WaitForExit(5000) }
}

# Apply the same material interface to an existing GLB skin and render its triangle pass.
$meshPack = Join-Path $reportDir 'mesh-pack'
New-Item -ItemType Directory -Force $meshPack | Out-Null
Copy-Item -LiteralPath 'content/animation/rigify.glb' -Destination "$meshPack/rigify.glb"
$world = Get-Content 'content/animation/world.json' -Raw | ConvertFrom-Json -AsHashtable
$world.capabilities.shader = 1
$character = $world.entities | Where-Object { $_.id -eq 'character' }
$character.components['render.material'] = @{id='mesh'}
$world.resources['render.shaders'] = @{materials=@{mesh=@{shader='unlit';tint=@(0.8,0.04,0.75,1.0)}}}
[IO.File]::WriteAllText("$meshPack/world.json", ($world | ConvertTo-Json -Depth 60))
[IO.File]::WriteAllText("$meshPack/manifest.json", (@{id=$world.id;version=1;world='world.json';capabilities=$world.capabilities} | ConvertTo-Json -Depth 10))
$mesh = Invoke-ShaderAxiom @('--db',"data/shader-mesh-$stamp.sqlite",'--pack',$meshPack,'play','--start','--port','0','--smoke-frames','32','--screenshot',"$reportDir/mesh.png")
if ($mesh.observation.shader.materials -ne 1 -or $mesh.observation.animation.players.character.fault) { throw 'Shader did not activate on the GLB' }

Add-Type -AssemblyName System.Drawing.Common
function Count-Pixels([string]$Path, [scriptblock]$Predicate) {
    $bitmap = [Drawing.Bitmap]::new($Path)
    try {
        $count = 0
        for ($y = [int]($bitmap.Height * 0.23); $y -lt $bitmap.Height * 0.64; $y += 2) {
            for ($x = [int]($bitmap.Width * 0.44); $x -lt $bitmap.Width * 0.56; $x += 2) {
                if (& $Predicate ($bitmap.GetPixel($x, $y))) { $count++ }
            }
        }
        $count
    } finally { $bitmap.Dispose() }
}
$redPixels = { param($p) $p.R -gt 140 -and $p.R -gt 1.8*$p.G -and $p.R -gt 2.0*$p.B }
$magentaPixels = { param($p) $p.R -gt 145 -and $p.B -gt 130 -and $p.G -lt 100 }
$before = Count-Pixels "$reportDir/before.png" $redPixels
$after = Count-Pixels "$reportDir/after.png" $redPixels
$triangles = Count-Pixels "$reportDir/mesh.png" $magentaPixels
if ($after -lt 100 -or $after -lt $before + 100 -or $triangles -lt 50) { throw "GPU color evidence failed: red $before -> $after, magenta mesh $triangles" }

$rtsDb = "data/shader-rts-$stamp.sqlite"
$rts = Invoke-ShaderAxiom @('--db',$rtsDb,'--pack','content/rts-bench','scenario','examples/rts-functional.scenario.json')
$rtsGpu = Invoke-ShaderAxiom @('--db',$rtsDb,'play','--start','--port','0','--smoke-frames','8','--screenshot',"$reportDir/rts.png")
if ($rts.observation.rts.summary.units_total -ne 66 -or $rtsGpu.observation.rts.summary.units_total -ne 64) { throw 'RTS regression' }
$undo = Invoke-ShaderAxiom @('--db',$db,'undo',"$($active.rev)",'--expected-rev',"$($failure.rev)")
if (-not $undo.ok -or (Get-FileHash -LiteralPath $exe).Hash -ne $hash) { throw 'Shader undo or executable consistency failed' }
[ordered]@{
    ok=$true;exe_sha256=$hash;gpu=$baseline.gpu;earlier_regressions=(-not $SkipEarlier);
    baseline=$initial;cpu_diagnostic=$invalid;valid_hot_reload=$active;failed_gpu_activation=$retained;
    mesh=$mesh.observation;undo=$undo;rts=$rts.observation.rts.summary;
    pixels=@{red_before=$before;red_after=$after;magenta_mesh=$triangles};
    screenshots=@{before="$reportDir/before.png";after="$reportDir/after.png";mesh="$reportDir/mesh.png";rts="$reportDir/rts.png"}
} | ConvertTo-Json -Depth 40 | Set-Content -Encoding utf8 "$reportDir/report.json"
Copy-Item -LiteralPath "$reportDir/report.json" -Destination 'artifacts/shader-validation.json'
Write-Host "PASS shader assets, live replacement, last-good GPU pipeline, mesh material and regressions. Report: $reportDir/report.json"
