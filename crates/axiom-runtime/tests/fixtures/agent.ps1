param([string]$Suite)
$ErrorActionPreference = 'Stop'
$agentSuite = Get-Content -LiteralPath $Suite -Raw | ConvertFrom-Json
$agentTurns = @{}
while ($null -ne ($agentLine = [Console]::ReadLine())) {
    $agentFrame = $agentLine | ConvertFrom-Json
    $agentTask = $agentSuite.tasks | Where-Object id -EQ $agentFrame.task_id
    $agentIndex = $agentTurns[$agentFrame.task_id]
    if ($null -eq $agentIndex) { $agentIndex = 0 }
    # Fixture metadata deliberately does not claim real provider tokens.
    $agentReply = @{usage=@{provider='fixture';model='deterministic';input_tokens=100;output_tokens=10;source='fixture'}}
    if ($agentIndex -lt $agentTask.plans.local.Count) { $agentReply.request=$agentTask.plans.local[$agentIndex] } else { $agentReply.done=$true }
    $agentTurns[$agentFrame.task_id] = $agentIndex + 1
    [Console]::WriteLine(($agentReply | ConvertTo-Json -Compress -Depth 50))
}
