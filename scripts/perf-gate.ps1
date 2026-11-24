# The speed gate with a re-time: runs the perf budget check (crates/engine/examples/perf.rs)
# and, when some check misses its budget, runs the whole check again, up to two more times.
# A check fails the gate only if it missed on all three runs; a check that missed once and
# passed on a retry is printed as "missed once, passed on retry" and does not fail it. A run
# that stops without printing its result (a panic, a crash) fails at once.
#
#   pwsh scripts/perf-gate.ps1 [rows]        (default 1,000,000 rows; use 10000000 for the full-size gate)
#
# The log of every run is kept under target/perf/gate-<run>.log.
param([int]$Rows = 1000000)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
cargo build --release -p natendo-engine --examples
if ($LASTEXITCODE -ne 0) { throw 'build failed' }
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
$exe = Join-Path $targetDir 'release\examples\perf.exe'
New-Item -ItemType Directory -Force -Path (Join-Path $root 'target\perf') | Out-Null

# The name of a check as printed on a FAIL line, without the numbers that change from run to run.
function Get-Missed([string[]]$lines) {
  $lines | Where-Object { $_ -match '^FAIL ' } | ForEach-Object {
    ($_ -replace '^FAIL ', '' -replace ': [0-9.]+ ms \(budget.*$', '' -replace ', [0-9]+ names/s', '')
  }
}

$firstMissed = @()
$stillMissing = $null
for ($run = 1; $run -le 3; $run++) {
  Write-Host "==> perf run $run of up to 3 ($Rows rows)"
  $log = Join-Path $root "target\perf\gate-$run.log"
  & $exe $Rows 2>&1 | Tee-Object -FilePath $log | Out-Null
  $code = $LASTEXITCODE
  $lines = Get-Content $log
  $summary = $lines | Where-Object { $_ -match 'performance budget missed' }
  $passes = @($lines | Where-Object { $_ -match '^PASS ' }).Count
  if ($code -ne 0 -and -not $summary) { Write-Host "perf stopped with exit $code before it finished (see $log)"; exit 2 }
  $missed = @(Get-Missed $lines)
  Write-Host "    $passes checks passed, $($missed.Count) missed"
  if ($run -eq 1) { $firstMissed = $missed; $stillMissing = $missed }
  else { $stillMissing = @($stillMissing | Where-Object { $missed -contains $_ }) }
  if ($stillMissing.Count -eq 0) { break }
}
$retried = @($firstMissed | Where-Object { $stillMissing -notcontains $_ })
foreach ($n in $retried) { Write-Host "missed once, passed on retry: $n" }
if ($stillMissing.Count -gt 0) {
  foreach ($n in $stillMissing) { Write-Host "MISSED ON EVERY RUN: $n" }
  Write-Host 'speed gate: FAILED (a check that misses on every run goes to a person)'
  exit 1
}
Write-Host 'speed gate: passed'
exit 0
