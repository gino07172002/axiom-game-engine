# Deliberately unresponsive adapter; the test host must terminate it after its one-second budget.
[Console]::ReadLine() | Out-Null
Start-Sleep -Seconds 30
