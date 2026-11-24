# Port guards shared by every script that writes through a loopback bridge it started itself
# (parity scripts, editor-smoke-test, shader-smoke-test, perf-live-edit). Dot-source it.
# A script refuses a port that already answers before it starts anything, and checks that the
# bridge belongs to the process it started before its first write: otherwise a running editor
# or game on that port (and its database) would receive the test edits.

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

# Process IDs listening on TCP port $P (empty when none). Parsed from `netstat -ano`, which
# works for a standard account (Get-NetTCPConnection can fail there with WMI access denied).
# A listening socket is the row whose foreign address is 0.0.0.0:0 or [::]:0, so the
# localized state column is not needed.
function Get-PortOwners([int]$P) {
    $lines = & netstat.exe -ano
    if ($LASTEXITCODE -ne 0) { throw "netstat -ano failed (exit $LASTEXITCODE); cannot check who owns port $P" }
    $owners = foreach ($line in $lines) {
        if ($line -match '^\s*TCP\s+\S+:(\d+)\s+(\S+)\s+\S+\s+(\d+)\s*$') {
            $port, $foreign, $owner = [int]$Matches[1], $Matches[2], [int]$Matches[3]
            if ($port -eq $P -and ($foreign -eq '0.0.0.0:0' -or $foreign -eq '[::]:0')) { $owner }
        }
    }
    @($owners | Sort-Object -Unique)
}

# The bridge on $P must belong to process $Id (the editor or game this script started) and to
# no other process; checked before the first write.
function Assert-BridgeOwner([int]$P, [int]$Id) {
    $owners = @(Get-PortOwners $P)
    if ($owners.Count -ne 1 -or $owners[0] -ne $Id) {
        throw "the bridge on port $P belongs to process(es) [$($owners -join ', ')], not to the process this script started (pid $Id); no write was sent."
    }
}
