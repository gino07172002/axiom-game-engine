# Shared helpers of the live editor <-> ACL parity scripts (editor-world-parity.ps1,
# street-editor-parity.ps1). Dot-source after $Port is set; Send-Editor talks to that port.
# Every path is absolute: the callers pass folders under their own repository root.

. "$PSScriptRoot/bridge-owner.ps1"

# One JSONL request to the loopback bridge on $Port; returns the parsed reply. The default read
# timeout covers ordinary requests; `screenshot` passes a longer one (see Save-Capture).
function Send-Editor($Request, [int]$TimeoutMs = 30000) {
    $client = [Net.Sockets.TcpClient]::new()
    try {
        $client.Connect('127.0.0.1', $Port)
        $client.ReceiveTimeout = $TimeoutMs
        $stream = $client.GetStream()
        $writer = [IO.StreamWriter]::new($stream, [Text.UTF8Encoding]::new($false), 1024, $true)
        $writer.AutoFlush = $true
        $writer.WriteLine(($Request | ConvertTo-Json -Compress -Depth 30))
        [IO.StreamReader]::new($stream, [Text.UTF8Encoding]::new($false)).ReadLine() | ConvertFrom-Json
    } finally { $client.Dispose() }
}

# A new, empty folder for this run under $Parent (never reused, so stale files cannot pass).
function New-RunFolder([string]$Parent) {
    $path = Join-Path $Parent ("run-{0}-{1}" -f (Get-Date -Format 'yyyyMMdd-HHmmss-fff'), $PID)
    if (Test-Path -LiteralPath $path) { throw "run folder $path already exists" }
    New-Item -ItemType Directory -Path $path | Out-Null
    return $path
}

# Captures the editor window to $Folder\$Name.png through the loopback `screenshot` op, which
# answers once the PNG is written (read back asynchronously, never on the main thread); fails
# unless the reply is ok and the file exists and is not empty. Returns the path.
# The bridge waits up to 32 s (SCREENSHOT_WAIT) plus a 1 s reply grace, so the read timeout is
# 40 s. The op refuses while another capture is in flight (unlike `editor_view {capture}`, it
# does not queue): that refusal is retried until a 60 s wall-clock deadline.
function Save-Capture([string]$Folder, [string]$Name) {
    $path = Join-Path $Folder "$Name.png"
    $deadline = [Diagnostics.Stopwatch]::StartNew()
    while ($true) {
        $shot = Send-Editor @{ op = 'screenshot'; path = $path } -TimeoutMs 40000
        $text = $shot | ConvertTo-Json -Depth 10 -Compress
        if ($shot.ok -or $text -notmatch 'already being captured' -or $deadline.Elapsed.TotalSeconds -ge 60) { break }
        Start-Sleep -Milliseconds 250
    }
    if (-not $shot.ok) { throw "screenshot ${Name}: $text" }
    if (-not (Test-Path -LiteralPath $path) -or (Get-Item -LiteralPath $path).Length -le 0) {
        throw "screenshot ${Name}: the reply was ok but $path is missing or empty"
    }
    return $path
}

# Exports $Db to $Path (which must not exist yet) and returns its SHA-256; throws on a failed
# export or a missing file.
function Export-World([string]$Exe, [string]$Db, [string]$Path) {
    if (Test-Path -LiteralPath $Path) { throw "export target $Path already exists" }
    $out = & $Exe --db $Db export $Path 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "export $Db failed: $out" }
    if (-not (Test-Path -LiteralPath $Path)) { throw "export $Db reported success but wrote no $Path" }
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
}

# Current revision of $Db; throws when the query fails.
function Get-WorldRev([string]$Exe, [string]$Db) {
    $out = & $Exe --db $Db query
    if ($LASTEXITCODE -ne 0) { throw "query $Db failed: $out" }
    return ($out | ConvertFrom-Json).rev
}
