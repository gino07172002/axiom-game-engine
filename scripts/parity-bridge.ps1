# Shared helpers of the live editor <-> ACL parity scripts (editor-world-parity.ps1,
# street-editor-parity.ps1). Dot-source after $Port is set; Send-Editor talks to that port.
# Every path is absolute: the callers pass folders under their own repository root.

# One JSONL request to the loopback bridge on $Port; returns the parsed reply.
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

# True when something already accepts connections on 127.0.0.1:$P.
function Test-PortAnswers([int]$P) {
    $client = [Net.Sockets.TcpClient]::new()
    try {
        $connect = $client.ConnectAsync('127.0.0.1', $P)
        return ($connect.Wait(500) -and $client.Connected)
    } catch { return $false } finally { $client.Dispose() }
}

# Refuses a port that already answers: a running editor or game there would receive the writes.
function Assert-PortFree([int]$P, [switch]$Allow) {
    if ($Allow) { return }
    if (Test-PortAnswers $P) {
        throw "port $P already answers (another editor or game?); nothing was started or written. Pick a free -Port (-AllowBusyPort skips only this pre-check)."
    }
}

# Process IDs listening on TCP port $P (empty when none).
function Get-PortOwners([int]$P) {
    @(Get-NetTCPConnection -LocalPort $P -State Listen -ErrorAction SilentlyContinue |
        ForEach-Object { [int]$_.OwningProcess } | Sort-Object -Unique)
}

# The bridge on $P must belong to process $Id (the editor this script started) and to no other
# process; checked before the first write.
function Assert-BridgeOwner([int]$P, [int]$Id) {
    $owners = @(Get-PortOwners $P)
    if ($owners.Count -ne 1 -or $owners[0] -ne $Id) {
        throw "the bridge on port $P belongs to process(es) [$($owners -join ', ')], not to the editor this script started (pid $Id); no write was sent."
    }
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
function Save-Capture([string]$Folder, [string]$Name) {
    $path = Join-Path $Folder "$Name.png"
    $shot = Send-Editor @{ op = 'screenshot'; path = $path }
    if (-not $shot.ok) { throw "screenshot ${Name}: $($shot | ConvertTo-Json -Depth 10 -Compress)" }
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
