# Runs a program and reports its peak working set in MB (polled every 20 ms) and its run time.
#   pwsh scripts/peak-mem.ps1 <exe> [args...]
param([Parameter(Mandatory)][string]$Exe, [Parameter(ValueFromRemainingArguments)][string[]]$Rest)
$p = Start-Process -FilePath $Exe -ArgumentList $Rest -PassThru -NoNewWindow -RedirectStandardOutput ([IO.Path]::GetTempFileName())
$peak = 0; $sw = [Diagnostics.Stopwatch]::StartNew()
while (-not $p.HasExited) {
    try { $p.Refresh(); if ($p.WorkingSet64 -gt $peak) { $peak = $p.WorkingSet64 } } catch {}
    Start-Sleep -Milliseconds 20
}
"{0} peak {1:N0} MB in {2:N1} s" -f (Split-Path $Exe -Leaf), ($peak / 1MB), $sw.Elapsed.TotalSeconds
