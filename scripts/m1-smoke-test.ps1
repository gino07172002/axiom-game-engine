param([int]$Port = 4272, [switch]$SkipM0)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
. "$PSScriptRoot/env.ps1"
$m1Exe = (Resolve-Path 'target/debug/axiom.exe').Path
$m1Hash = (Get-FileHash -LiteralPath $m1Exe -Algorithm SHA256).Hash
$m1Stamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'
$m1Dir = Join-Path $PWD "artifacts/m1-$m1Stamp"
$m1Db = Join-Path $PWD "data/m1-$m1Stamp.sqlite"
New-Item -ItemType Directory -Force -Path $m1Dir | Out-Null
function Invoke-M1([string[]]$M1Args) {
    $m1Raw = & $m1Exe @M1Args
    if ($LASTEXITCODE -ne 0) { throw "axiom failed: $M1Args" }
    return ($m1Raw | ConvertFrom-Json)
}
function Invoke-M1Wire($M1Request) {
    $m1Client = [System.Net.Sockets.TcpClient]::new()
    try {
        $m1Client.Connect('127.0.0.1', $Port)
        $m1Client.ReceiveTimeout = 5000
        $m1Stream = $m1Client.GetStream()
        $m1Writer = [System.IO.StreamWriter]::new($m1Stream, [System.Text.UTF8Encoding]::new($false), 1024, $true)
        $m1Writer.AutoFlush = $true
        $m1Writer.WriteLine(($M1Request | ConvertTo-Json -Compress -Depth 30))
        $m1Reader = [System.IO.StreamReader]::new($m1Stream)
        return ($m1Reader.ReadLine() | ConvertFrom-Json)
    } finally { $m1Client.Dispose() }
}
if (-not $SkipM0) {
    & "$PSScriptRoot/m0-smoke-test.ps1" -Port ($Port + 1)
}
$m1Verify = Invoke-M1 @('--db',$m1Db,'--pack','content/adv','verify','--ticks','120')
$m1Reachability = Invoke-M1 @('--db',$m1Db,'narrative-check','--budget','4096')
if (-not $m1Reachability.complete -or @($m1Reachability.reachable_endings.PSObject.Properties).Count -ne 3) { throw 'Three endings were not proven reachable' }
$m1Short = Invoke-M1 @('--db',$m1Db,'narrative-check','--budget','1')
if ($m1Short.complete) { throw 'Exhausted reachability search claimed proof' }
$m1Scenarios = [ordered]@{}
foreach ($m1Name in @('rescue','departure','stranded','english')) {
    $m1Scenarios[$m1Name] = Invoke-M1 @('--db',$m1Db,'scenario',"examples/adv-$m1Name.scenario.json")
}
$m1Before = Invoke-M1 @('--db',$m1Db,'query')
$m1Preview = Invoke-M1 @('--db',$m1Db,'preview','examples/adv-branch-edit.json')
$m1AfterPreview = Invoke-M1 @('--db',$m1Db,'query')
if ($m1Before.rev -ne $m1AfterPreview.rev) { throw 'Preview modified history' }
$m1Apply = Invoke-M1 @('--db',$m1Db,'apply','examples/adv-branch-edit.json')
$m1AfterEdit = Invoke-M1 @('--db',$m1Db,'narrative-check')
if (-not $m1AfterEdit.complete -or @($m1AfterEdit.reachable_endings.PSObject.Properties).Count -ne 3) { throw 'Data-only edit broke branch reachability' }
$m1Undo = Invoke-M1 @('--db',$m1Db,'undo','1','--expected-rev','2')

$m1Screens = [ordered]@{}
$m1Screens['arrival'] = Invoke-M1 @('--db',$m1Db,'play','--port','0','--start','--smoke-frames','8','--screenshot',"$m1Dir/arrival.png")
foreach ($m1Slot in @('before-key','key','ending-rescue','ending-departure','ending-stranded','english')) {
    $m1Screens[$m1Slot] = Invoke-M1 @('--db',$m1Db,'play','--port','0','--start','--load-slot',$m1Slot,'--smoke-frames','8','--screenshot',"$m1Dir/$m1Slot.png")
}
if ($m1Screens['key'].observation.gameplay.variables.has_key -ne $true -or $m1Screens['ending-rescue'].observation.gameplay.ending -ne 'rescue') { throw 'Native save loading failed' }
if ($m1Screens['before-key'].observation.gameplay.choices.id -contains 'open') { throw 'Door option visible without key' }

$m1Probe = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback,$Port)
try { $m1Probe.Start() } finally { $m1Probe.Stop() }
$m1Process = Start-Process -FilePath $m1Exe -ArgumentList ('--db "{0}" play --port {1} --smoke-frames 6000' -f $m1Db,$Port) -WorkingDirectory $PWD -WindowStyle Hidden -PassThru -RedirectStandardOutput "$m1Dir/live.log" -RedirectStandardError "$m1Dir/live-errors.log"
try {
    $m1Ready = $false
    for ($m1Try=0;$m1Try -lt 50;$m1Try++) {
        try { $m1Observed = Invoke-M1Wire @{op='observe'}; $m1Ready = $m1Observed.ok; if ($m1Ready) { break } } catch { }
        if ($m1Process.HasExited) { throw 'Live ADV exited on startup' }
        Start-Sleep -Milliseconds 200
    }
    if (-not $m1Ready) { throw 'Live ADV did not become ready' }
    $m1Help = Invoke-M1 @('--connect',"127.0.0.1:$Port",'command','examples/adv-help.command.json','--expected-rev','3')
    $m1Denied = Invoke-M1Wire @{op='command';expected_rev=3;command=@{kind='choose';node='workshop';choice='open'}}
    if ($m1Denied.ok) { throw 'Hidden choice was accepted' }
    $m1Inspect = Invoke-M1 @('--connect',"127.0.0.1:$Port",'command','examples/adv-inspect.command.json')
    $m1Save = Invoke-M1 @('--connect',"127.0.0.1:$Port",'save','live')
    $m1Return = Invoke-M1Wire @{op='command';command=@{kind='choose';node='clue';choice='return'}}
    $m1Open = Invoke-M1Wire @{op='command';command=@{kind='choose';node='workshop';choice='open'}}
    if (-not $m1Open.ok -or $m1Open.gameplay.ending -ne 'rescue') { throw 'Live ADV branch failed' }
    $m1Load = Invoke-M1 @('--connect',"127.0.0.1:$Port",'load','live')
    $m1LiveEdit = Get-Content 'examples/adv-branch-edit.json' -Raw | ConvertFrom-Json
    $m1LiveEdit.expected_rev = 3
    $m1LiveApply = Invoke-M1Wire $m1LiveEdit
    $m1Synchronized = $false
    for ($m1Try=0;$m1Try -lt 50;$m1Try++) {
        $m1Observed = Invoke-M1Wire @{op='observe'}
        if ($m1Observed.observation.rev -eq 4 -and $m1Observed.observation.gameplay.node -eq 'clue' -and $m1Observed.observation.gameplay.variables.has_key) { $m1Synchronized=$true; break }
        Start-Sleep -Milliseconds 100
    }
    if (-not $m1Synchronized) { throw 'Live data edit lost narrative progress' }
    $m1Mismatch = Invoke-M1Wire @{op='load';slot='live'}
    if ($m1Mismatch.ok) { throw 'Incompatible content save was silently accepted' }
    $m1LiveUndo = Invoke-M1Wire @{op='undo';rev=1;expected_rev=4}
    $m1Restored = Invoke-M1Wire @{op='load';slot='live'}
    if (-not $m1Restored.ok -or $m1Restored.rev -ne 5) { throw 'Save failed after restoring matching content' }
    $m1Final = Invoke-M1Wire @{op='observe'}
    if ($m1Final.observation.gameplay.node -ne 'clue' -or $m1Final.observation.gameplay.variables.clues -ne 1) { throw 'Loaded flags/node did not survive live pipeline' }
} finally { if (-not $m1Process.HasExited) { Stop-Process -Id $m1Process.Id } }

if ((Get-FileHash -LiteralPath $m1Exe -Algorithm SHA256).Hash -ne $m1Hash) { throw 'Executable changed during content operations' }
Copy-Item -LiteralPath "$m1Dir/arrival.png" -Destination 'artifacts/m1-adv.png'
Copy-Item -LiteralPath "$m1Dir/key.png" -Destination 'artifacts/m1-key.png'
Copy-Item -LiteralPath "$m1Dir/ending-rescue.png" -Destination 'artifacts/m1-ending.png'
Copy-Item -LiteralPath "$m1Dir/english.png" -Destination 'artifacts/m1-english.png'
$m1Report = [ordered]@{ok=$true;same_executable=$true;executable_sha256=$m1Hash;m0_included=(-not $SkipM0);verify=$m1Verify;reachability=$m1Reachability;limited_search=$m1Short;scenarios=$m1Scenarios;preview=$m1Preview;apply=$m1Apply;undo=$m1Undo;screens=$m1Screens;live=@{help=$m1Help;hidden_choice=$m1Denied;inspect=$m1Inspect;save=$m1Save;open=$m1Open;load=$m1Load;apply=$m1LiveApply;observed=$m1Observed;incompatible_save=$m1Mismatch;undo=$m1LiveUndo;restored=$m1Restored;final=$m1Final}}
$m1Report | ConvertTo-Json -Depth 50 | Set-Content -Encoding utf8 "$m1Dir/report.json"
Copy-Item -LiteralPath "$m1Dir/report.json" -Destination 'artifacts/m1-validation.json'
Write-Host "PASS M1: ADV / 3 endings / bounded reachability / UI bindings / locale / checkpoints / live commands / data-only edit / FPS regression. Report: $m1Dir/report.json"
