# Local CI: same checks as .github/workflows/ci.yml. Run from the repo root:
#   pwsh scripts/ci.ps1          # all checks
#   pwsh scripts/ci.ps1 -UiOnly  # skip Rust (e.g. before rustup is installed)
param([switch]$UiOnly)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)

function Step($name, [scriptblock]$cmd) {
    Write-Host "==> $name" -ForegroundColor Cyan
    & $cmd
    if ($LASTEXITCODE -ne 0) { throw "$name failed (exit $LASTEXITCODE)" }
}

if (-not (Test-Path node_modules)) { Step 'npm ci' { npm ci } }
Step 'ui: type check' { npm run check -w ui }
Step 'ui: tests'      { npm test -w ui }
Step 'ui: build'      { npm run build -w ui }

if (-not $UiOnly) {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw 'cargo not found: install rustup, or pass -UiOnly'
    }
    Step 'rust: fmt'    { cargo fmt --all -- --check }
    Step 'rust: clippy' { cargo clippy --workspace --all-targets -- -D warnings }
    Step 'rust: tests'  { cargo test --workspace }
}
Write-Host 'All checks passed.' -ForegroundColor Green
